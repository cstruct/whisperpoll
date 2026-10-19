//! Where polls live, so any number of server machines can serve them.
//!
//! * **Memory** (no `REDIS_URL`): one process holds everything. Fine for
//!   development; polls are lost on restart.
//! * **Redis** (`REDIS_URL` set): every poll is a JSON document with a
//!   one-week TTL, refreshed on each change. Changes are serialized per poll
//!   with a short Redis lock, announced over pub/sub so connections on other
//!   machines refresh, and presence (who has a live connection) is a sorted
//!   set of connection expiries, kept fresh by heartbeats. A crashed machine's
//!   connections simply expire.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use redis::AsyncCommands;
use tokio::sync::broadcast;

use crate::room::{random_token, Online, Room};

/// How long an untouched poll is kept.
pub const TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long a connection counts as online without a heartbeat.
pub const PRESENCE_TTL: Duration = Duration::from_secs(60);
/// How often sessions renew their presence.
pub const HEARTBEAT: Duration = Duration::from_secs(20);

const LOCK_TTL_MS: u64 = 5_000;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub struct Store {
    backend: Backend,
    /// Local subscribers per poll, woken when the poll changes anywhere.
    changed: Arc<Mutex<HashMap<String, broadcast::Sender<()>>>>,
}

#[derive(Clone)]
enum Backend {
    Memory(Arc<Mutex<Memory>>),
    Redis(redis::aio::ConnectionManager),
}

#[derive(Default)]
struct Memory {
    rooms: HashMap<String, Room>,
    /// poll → connection → (participant, expiry)
    presence: HashMap<String, HashMap<String, (usize, Instant)>>,
}

fn poll_key(id: &str) -> String {
    format!("whisperpoll:poll:{id}")
}

fn lock_key(id: &str) -> String {
    format!("whisperpoll:lock:{id}")
}

fn presence_key(id: &str) -> String {
    format!("whisperpoll:presence:{id}")
}

fn channel(id: &str) -> String {
    format!("whisperpoll:changed:{id}")
}

const CHANNEL_PATTERN: &str = "whisperpoll:changed:*";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn redis_err(e: redis::RedisError) -> String {
    format!("storage: {e}")
}

impl Store {
    /// Redis if `REDIS_URL` is set, memory otherwise.
    pub async fn from_env() -> Result<Store> {
        let changed = Arc::new(Mutex::new(HashMap::new()));
        let Ok(url) = std::env::var("REDIS_URL") else {
            return Ok(Store {
                backend: Backend::Memory(Arc::default()),
                changed,
            });
        };
        let client = redis::Client::open(url).map_err(redis_err)?;
        let conn = client.get_connection_manager().await.map_err(redis_err)?;
        tokio::spawn(relay_changes(client, changed.clone()));
        Ok(Store {
            backend: Backend::Redis(conn),
            changed,
        })
    }

    pub fn describe(&self) -> &'static str {
        match self.backend {
            Backend::Memory(_) => "in memory",
            Backend::Redis(_) => "in Redis",
        }
    }

    /// Stores a new poll.
    pub async fn create(&self, id: &str, room: &Room) -> Result<()> {
        match &self.backend {
            Backend::Memory(m) => {
                let room = serde_json::from_str(&serde_json::to_string(room).unwrap()).unwrap();
                m.lock().unwrap().rooms.insert(id.to_string(), room);
                Ok(())
            }
            Backend::Redis(conn) => {
                let json = serde_json::to_string(room).map_err(|e| e.to_string())?;
                let mut conn = conn.clone();
                let _: () = conn
                    .set_ex(poll_key(id), json, TTL.as_secs())
                    .await
                    .map_err(redis_err)?;
                Ok(())
            }
        }
    }

    /// Reads a poll; `None` if it doesn't exist (or expired).
    pub async fn read<T>(
        &self,
        id: &str,
        f: impl FnOnce(&Room, &Online) -> T,
    ) -> Result<Option<T>> {
        match &self.backend {
            Backend::Memory(m) => {
                let mut m = m.lock().unwrap();
                let online = memory_online(&mut m, id);
                Ok(m.rooms.get(id).map(|room| f(room, &online)))
            }
            Backend::Redis(conn) => {
                let mut conn = conn.clone();
                let Some(room) = load(&mut conn, id).await? else {
                    return Ok(None);
                };
                let online = redis_online(&mut conn, id).await?;
                Ok(Some(f(&room, &online)))
            }
        }
    }

    /// Changes a poll atomically (across machines) and tells every
    /// connection to it. `None` if it doesn't exist.
    pub async fn update<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut Room, &Online) -> T,
    ) -> Result<Option<T>> {
        let result = match &self.backend {
            Backend::Memory(m) => {
                let mut m = m.lock().unwrap();
                let online = memory_online(&mut m, id);
                m.rooms.get_mut(id).map(|room| f(room, &online))
            }
            Backend::Redis(conn) => {
                let mut conn = conn.clone();
                let token = lock(&mut conn, id).await?;
                let result: Result<Option<T>> = async {
                    let Some(mut room) = load(&mut conn, id).await? else {
                        return Ok(None);
                    };
                    let online = redis_online(&mut conn, id).await?;
                    let out = f(&mut room, &online);
                    let json = serde_json::to_string(&room).map_err(|e| e.to_string())?;
                    let _: () = conn
                        .set_ex(poll_key(id), json, TTL.as_secs())
                        .await
                        .map_err(redis_err)?;
                    Ok(Some(out))
                }
                .await;
                unlock(&mut conn, id, &token).await;
                result?
            }
        };
        if result.is_some() {
            self.notify(id).await;
        }
        Ok(result)
    }

    /// Marks one connection of participant `who` as online (or renews it),
    /// or as gone.
    pub async fn set_online(&self, id: &str, who: usize, conn_id: &str, online: bool) {
        match &self.backend {
            Backend::Memory(m) => {
                let mut m = m.lock().unwrap();
                let conns = m.presence.entry(id.to_string()).or_default();
                if online {
                    conns.insert(conn_id.to_string(), (who, Instant::now() + PRESENCE_TTL));
                } else {
                    conns.remove(conn_id);
                }
            }
            Backend::Redis(conn) => {
                let mut conn = conn.clone();
                let member = format!("{who}:{conn_id}");
                let key = presence_key(id);
                let _ = if online {
                    let expiry = now_ms() + PRESENCE_TTL.as_millis() as u64;
                    redis::pipe()
                        .zadd(&key, member, expiry)
                        .ignore()
                        .expire(&key, TTL.as_secs() as i64)
                        .ignore()
                        .query_async::<()>(&mut conn)
                        .await
                } else {
                    conn.zrem::<_, _, ()>(&key, member).await
                };
            }
        }
    }

    /// Wakes every connection to poll `id`, on every machine.
    pub async fn notify(&self, id: &str) {
        match &self.backend {
            Backend::Memory(_) => wake(&self.changed, id),
            Backend::Redis(conn) => {
                let mut conn = conn.clone();
                let _ = conn.publish::<_, _, ()>(channel(id), "").await;
            }
        }
    }

    /// Change notifications for poll `id` on this machine.
    pub fn subscribe(&self, id: &str) -> broadcast::Receiver<()> {
        let mut changed = self.changed.lock().unwrap();
        changed.retain(|_, tx| tx.receiver_count() > 0);
        changed
            .entry(id.to_string())
            .or_insert_with(|| broadcast::channel(64).0)
            .subscribe()
    }
}

fn wake(changed: &Mutex<HashMap<String, broadcast::Sender<()>>>, id: &str) {
    if let Some(tx) = changed.lock().unwrap().get(id) {
        let _ = tx.send(());
    }
}

fn memory_online(m: &mut Memory, id: &str) -> Online {
    let now = Instant::now();
    let Some(conns) = m.presence.get_mut(id) else {
        return Online::new();
    };
    conns.retain(|_, (_, expiry)| *expiry > now);
    conns.values().map(|(who, _)| *who).collect()
}

async fn load(conn: &mut redis::aio::ConnectionManager, id: &str) -> Result<Option<Room>> {
    let json: Option<String> = conn.get(poll_key(id)).await.map_err(redis_err)?;
    json.map(|j| serde_json::from_str(&j).map_err(|e| format!("corrupt poll {id}: {e}")))
        .transpose()
}

async fn redis_online(conn: &mut redis::aio::ConnectionManager, id: &str) -> Result<Online> {
    let key = presence_key(id);
    let now = now_ms();
    let _: () = conn
        .zrembyscore(&key, "-inf", now)
        .await
        .map_err(redis_err)?;
    let members: Vec<String> = conn.zrange(&key, 0, -1).await.map_err(redis_err)?;
    Ok(members
        .iter()
        .filter_map(|m| m.split_once(':')?.0.parse().ok())
        .collect())
}

/// Takes the poll's lock, waiting for another machine's change to finish.
async fn lock(conn: &mut redis::aio::ConnectionManager, id: &str) -> Result<String> {
    let token = random_token();
    let deadline = Instant::now() + Duration::from_millis(LOCK_TTL_MS);
    loop {
        let acquired: bool = redis::cmd("SET")
            .arg(lock_key(id))
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(LOCK_TTL_MS)
            .query_async::<Option<String>>(conn)
            .await
            .map_err(redis_err)?
            .is_some();
        if acquired {
            return Ok(token);
        }
        if Instant::now() > deadline {
            return Err("the poll is busy, try again".into());
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

/// Releases the lock only if it's still ours.
async fn unlock(conn: &mut redis::aio::ConnectionManager, id: &str, token: &str) {
    let script = redis::Script::new(
        r#"if redis.call("GET", KEYS[1]) == ARGV[1] then return redis.call("DEL", KEYS[1]) else return 0 end"#,
    );
    let _ = script
        .key(lock_key(id))
        .arg(token)
        .invoke_async::<i64>(conn)
        .await;
}

/// Forwards change announcements from Redis to this machine's connections,
/// reconnecting if the subscription drops.
async fn relay_changes(
    client: redis::Client,
    changed: Arc<Mutex<HashMap<String, broadcast::Sender<()>>>>,
) {
    loop {
        match client.get_async_pubsub().await {
            Ok(mut pubsub) => {
                if pubsub.psubscribe(CHANNEL_PATTERN).await.is_ok() {
                    let mut messages = pubsub.on_message();
                    while let Some(msg) = messages.next().await {
                        let name = msg.get_channel_name();
                        if let Some(id) = name.strip_prefix("whisperpoll:changed:") {
                            wake(&changed, id);
                        }
                    }
                }
            }
            Err(e) => eprintln!("redis pubsub: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

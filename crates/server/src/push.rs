//! Push notifications to participants' devices.
//!
//! Which notifier is used comes from `WHISPERPOLL_PUSH`:
//!
//! * `log` (default): print what would be sent.
//! * `simulator:<bundle id>`: deliver to the booted iOS simulator with
//!   `xcrun simctl push`, for testing the app's push handling without an
//!   Apple developer account. Development on macOS only.
//! * `none`: drop them.
//!
//! Real delivery through Apple's push service needs an APNs key from a paid
//! Apple developer account and slots in as another [`Notifier`].

use std::sync::Arc;

use serde_json::json;
use tokio::io::AsyncWriteExt;
use whisperpoll_protocol::PushPlatform;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub platform: PushPlatform,
    pub token: String,
}

/// One notification. Carries only the poll id, never invite tokens: pushes
/// travel through Apple's servers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Push {
    pub poll: String,
    pub title: String,
    pub body: String,
    /// Also wake the app in the background, so it can do its trustee step
    /// without the person opening it.
    pub wake: bool,
}

impl Push {
    pub fn apns_payload(&self) -> serde_json::Value {
        let mut aps = json!({
            "alert": { "title": self.title, "body": self.body },
            "sound": "default",
            // Groups a poll's notifications, and lets the app clear them.
            "thread-id": self.poll,
        });
        if self.wake {
            aps["content-available"] = json!(1);
        }
        json!({ "aps": aps, "poll": self.poll })
    }
}

pub trait Notifier: Send + Sync {
    fn send(&self, devices: &[Device], push: &Push);
}

pub fn from_env() -> Arc<dyn Notifier> {
    let setting = std::env::var("WHISPERPOLL_PUSH").unwrap_or_else(|_| "log".into());
    match setting.split_once(':') {
        Some(("simulator", bundle_id)) => Arc::new(Simulator {
            bundle_id: bundle_id.to_string(),
        }),
        _ if setting == "none" => Arc::new(Silent),
        _ => Arc::new(Log),
    }
}

struct Log;

impl Notifier for Log {
    fn send(&self, devices: &[Device], push: &Push) {
        println!(
            "push to {} device(s) for poll {}: {} — {}{}",
            devices.len(),
            push.poll,
            push.title,
            push.body,
            if push.wake { " (wakes app)" } else { "" },
        );
    }
}

struct Silent;

impl Notifier for Silent {
    fn send(&self, _: &[Device], _: &Push) {}
}

struct Simulator {
    bundle_id: String,
}

impl Notifier for Simulator {
    fn send(&self, devices: &[Device], push: &Push) {
        Log.send(devices, push);
        let payload = push.apns_payload().to_string();
        let bundle_id = self.bundle_id.clone();
        tokio::spawn(async move {
            let child = tokio::process::Command::new("xcrun")
                .args(["simctl", "push", "booted", &bundle_id, "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn();
            let Ok(mut child) = child else {
                eprintln!("push: couldn't run xcrun simctl");
                return;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(payload.as_bytes()).await;
            }
            let _ = child.wait().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_pushes_are_marked_content_available() {
        let mut push = Push {
            poll: "abc".into(),
            title: "Dinner".into(),
            body: "Help set up the key".into(),
            wake: true,
        };
        let payload = push.apns_payload();
        assert_eq!(payload["aps"]["content-available"], 1);
        assert_eq!(payload["poll"], "abc");
        assert_eq!(payload["aps"]["thread-id"], "abc");
        push.wake = false;
        assert!(push.apns_payload()["aps"]
            .get("content-available")
            .is_none());
    }
}

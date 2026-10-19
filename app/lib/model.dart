// Read-only accessors over the server's `PollView` JSON, for display. The
// cryptographic parts are never interpreted here: the raw JSON goes back to
// Rust (see `src/rust/api/poll.dart`) for anything that needs them.

const tiers = ['Prefer', 'OK', 'If needed'];

enum Phase { inviting, gathering, keyCeremony, voting, tallying, done, failed }

class Participant {
  Participant(Map<String, dynamic> j)
      : name = j['name'] as String,
        creator = j['creator'] as bool,
        online = j['online'] as bool,
        registered = j['registration'] != null,
        hasBallot = j['has_ballot'] as bool,
        voted = j['voted'] as bool;

  final String name;
  final bool creator;
  final bool online;
  final bool registered;
  final bool hasBallot;
  final bool voted;
}

class CeremonyInfo {
  CeremonyInfo(Map<String, dynamic> j)
      : attempt = j['attempt'] as int,
        threshold = j['threshold'] as int,
        trustees = (j['trustees'] as List).cast<int>(),
        dealtBy = {for (final d in j['dealings'] as List) (d as Map)['dealer'] as int},
        checked = (j['checked'] as List).cast<bool>();

  final int attempt;
  final int threshold;

  /// Participant index of each trustee; trustee `j` is `trustees[j - 1]`.
  final List<int> trustees;

  /// Trustee indices (1-based) that have dealt.
  final Set<int> dealtBy;
  final List<bool> checked;

  bool get allDealt => dealtBy.length == trustees.length;
}

class RoundInfo {
  RoundInfo(Map<String, dynamic> j)
      : decryptedBy = {for (final p in j['partials'] as List) (p as Map)['trustee'] as int},
        hasTally = j['tally'] != null,
        winner = j['winner'] as int?;

  /// Trustee indices (1-based) that have decrypted.
  final Set<int> decryptedBy;
  final bool hasTally;
  final int? winner;
}

class PollView {
  PollView(this.json, this.raw);

  final Map<String, dynamic> json;

  /// The same state as JSON text, for the Rust side.
  final String raw;

  String get id => json['id'] as String;
  String get title => json['title'] as String;
  int get me => json['me'] as int;
  int get days => json['days'] as int;

  Phase get phase {
    final p = json['phase'];
    return switch (p) {
      'Inviting' => Phase.inviting,
      'Gathering' => Phase.gathering,
      'KeyCeremony' => Phase.keyCeremony,
      'Voting' => Phase.voting,
      'Tallying' => Phase.tallying,
      'Done' => Phase.done,
      _ => Phase.failed,
    };
  }

  String? get failure {
    final p = json['phase'];
    return p is Map ? (p['Failed'] as Map)['reason'] as String : null;
  }

  List<Participant> get participants =>
      [for (final p in json['participants'] as List) Participant(p as Map<String, dynamic>)];

  Participant get mine => participants[me];
  bool get isCreator => mine.creator;

  /// The creator's invite links, as (name, token).
  List<(String, String)> get invites => [
        for (final i in json['invites'] as List)
          ((i as Map)['name'] as String, i['token'] as String)
      ];

  /// Suggested slots, earliest first, as (canonical slot text, suggested by).
  List<(String, String)> get slots => [
        for (final d in json['slots'] as List) ((d as Map)['slot'] as String, d['by'] as String)
      ];

  CeremonyInfo? get ceremony {
    final c = json['ceremony'];
    return c == null ? null : CeremonyInfo(c as Map<String, dynamic>);
  }

  List<String> get options => (json['options'] as List).cast<String>();
  List<RoundInfo> get rounds =>
      [for (final r in json['rounds'] as List) RoundInfo(r as Map<String, dynamic>)];

  /// Options picked by finished rounds.
  List<int> get picked => [for (final r in rounds) if (r.winner != null) r.winner!];
}

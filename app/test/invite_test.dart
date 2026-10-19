import 'package:flutter_test/flutter_test.dart';
import 'package:whisperpoll/storage.dart';

void main() {
  test('parses the web invite link', () {
    final invite = Invite.parse('https://whisperpoll.fly.dev/p/abc123?t=deadbeef')!;
    expect(invite.server, 'https://whisperpoll.fly.dev');
    expect(invite.poll, 'abc123');
    expect(invite.token, 'deadbeef');
    expect(invite.socketUrl.toString(), 'wss://whisperpoll.fly.dev/ws/abc123?t=deadbeef');
    expect(invite.checkUrl.toString(),
        'https://whisperpoll.fly.dev/api/polls/abc123/check?t=deadbeef');
  });

  test('parses a local development link', () {
    final invite = Invite.parse('  http://localhost:3000/p/abc?t=tok  ')!;
    expect(invite.server, 'http://localhost:3000');
    expect(invite.socketUrl.toString(), 'ws://localhost:3000/ws/abc?t=tok');
  });

  test("parses the app's own link", () {
    final invite = Invite.parse(
        'whisperpoll://p/abc?t=tok&server=https%3A%2F%2Fwhisperpoll.fly.dev')!;
    expect(invite.server, 'https://whisperpoll.fly.dev');
    expect(invite.poll, 'abc');
    expect(invite.webUrl.toString(), 'https://whisperpoll.fly.dev/p/abc?t=tok');
  });

  test('rejects anything else', () {
    for (final bad in [
      '',
      'hello',
      'https://whisperpoll.fly.dev/p/abc',
      'https://whisperpoll.fly.dev/x/abc?t=tok',
      'ftp://host/p/abc?t=tok',
      'whisperpoll://p/abc?t=tok',
    ]) {
      expect(Invite.parse(bad), isNull, reason: bad);
    }
  });
}

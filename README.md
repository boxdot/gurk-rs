# gurk 🥒
![CI][ci-badge] [![chat][chat-badge]][chat-link]

[Signal Messenger] client for terminal.

![screenshot](screenshot.png)

## Usage

Install from source:

```shell
cargo install --git https://github.com/boxdot/gurk-rs
```

Run

```
gurk
```

On the first run, this will open a QR code in your favorite image viewer, such that you can link the
device. This will also create a configuration file at the default [config
location][config-location]. For the configuration directives, see [`src/config.rs`].

## Chat

[![chat-qr](chat-qr.png)][chat-link]

## Features

* [ ] Store data in the db from [`presage`]'s `Manager` instead of a JSON file.
* [ ] Encrypt the storage by default.
* [x] Notifications over dbus or similar.
* [x] Scrolling of messages.
* [x] Reply functionality to a single message.
* [ ] Mouse navigation (works for channels, missing for the messages list).
* [ ] Search of messages/chats. Add quick switch between chats by name.
* [ ] Multiline messages; the `Enter` key sends the message.
* [ ] Viewing/sending of attachments.
* [ ] Support for blocked contacts/groups.
* [ ] Reactions with emojis.

## License

 * GNU Affero General Public License v3 only ([AGPL-3.0-only](LICENSE-AGPL-3.0) or
   https://www.gnu.org/licenses/agpl-3.0.en.html)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this document by you, as defined in the AGPL-3.0-only license,
shall be licensed as above, without any additional terms or conditions.

[Signal Messenger]: https://signal.org
[`presage`]: https://github.com/whisperfish/presage
[`src/config.rs`]: https://github.com/boxdot/gurk-rs/blob/master/src/config.rs
[chat-badge]: https://img.shields.io/badge/chat-on%20signal-brightgreen?logo=signal
[ci-badge]: https://github.com/boxdot/gurk-rs/workflows/CI/badge.svg
[chat-link]: https://signal.group/#CjQKILaqQTWUZks14mPRSn0m0zyU9A-buNMG6haQBmWrxJHeEhCc7HLIwCFZRNDw63MWj-fA
[config-location]: https://docs.rs/dirs/3.0.2/dirs/fn.config_dir.html

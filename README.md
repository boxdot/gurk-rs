# gurk 🥒
![CI][ci-badge] [![chat][chat-badge]][chat-link]

[Signal Messenger] client for terminal.

![screenshot](screenshot.png)

## Usage

You need to download and install [`signal-cli`], such that it is found in your `PATH`.

1. Download and install `signal-cli`
2. Follow the instructions at
   https://github.com/AsamK/signal-cli/wiki/Linking-other-devices-(Provisioning) to link
   `signal-cli` to your phone/device.
3. Install gurk with `cargo install gurk`
   To enable D-Bus notifications for new messages, run `cargo install --feature notifications gurk` instead.
4. Drop a config file with the following context
    ```
    [user]
    name = "Your user name"
    phone_number = "Your phone number used in Signal"
    ```

  in one of the following locations:

1. `$XDG_CONFIG_HOME/gurk/gurk.toml`
2. `$XDG_CONFIG_HOME/gurk.yml`
3. `$HOME/.config/gurk/gurk.toml`
4. `$HOME/.gurk.toml`

  For more config options, see [`src/config.rs`].

5. Run `gurk`

At the first run, `gurk` will sync groups and contacts.

## Chat

[![chat-qr](chat-qr.png)][chat-link]

## Missing features / known issues

* Use a simple database (like sqlite, sled, leveldb) for storing messages, contacts, etc... instead
  of a JSON file.
* Add optional Gnome notifications over dbus.
* Add scrolling of messages.
* Add reply functionality to a single message.
* Add mouse navigation.
* Add search of messages/chats. Add quick switch between chats by name.
* It is not possible to send multiline messages, since the `Enter` key sends the messages. Add a
  shortcut or a mode for typing multiline messages.
* Add sending of attachments.
* Add support for blocked contacts/groups.

The communication with the Signal backend is implemented via [`signal-cli`]. It provides some
functionality like lookup of group/contact name only over the dbus interface. Therefore, `gurk` only
works on Linux.

## Native implementation

There is ongoing effort to bring a native implementation to Rust of Signal messaging based on
[`libsignal-protocol-c`]. We have an experimental branch `native-client` which diverged quite far
from the implementation on master. The goal is to have at least the same feature we have on master.
After that we either merge the branch, reimplement it on master, or provide both clients as options.

For reference, check the following crates:

* [`libsignal-protocol-c`]: official Signal protocol implementation in C
* [`libsignal-protocol-rs`]: Rust bindings to the C library
* [`libsignal-service-rs`]: port of the official Java/? library
* [`presage`]: idiomatic Rust Signal client

## License

 * GNU Affero General Public License v3 only ([AGPL-3.0-only](LICENSE-AGPL-3.0) or
   https://www.gnu.org/licenses/agpl-3.0.en.html)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this document by you, as defined in the AGPL-3.0-only license,
shall be licensed as above, without any additional terms or conditions.

[Signal Messenger]: https://signal.org
[`signal-cli`]: https://github.com/AsamK/signal-cli
[`libsignal-service-rs`]: https://github.com/Michael-F-Bryan/libsignal-service-rs
[`libsignal-protocol-rs`]: https://github.com/Michael-F-Bryan/libsignal-protocol-rs
[`libsignal-protocol-c`]: https://github.com/signalapp/libsignal-protocol-c
[`presage`]: https://github.com/gferon/presage
[`src/config.rs`]: https://github.com/boxdot/gurk-rs/blob/master/src/config.rs
[chat-badge]: https://img.shields.io/badge/chat-on%20signal-brightgreen?logo=signal
[ci-badge]: https://github.com/boxdot/gurk-rs/workflows/CI/badge.svg
[chat-link]: https://signal.group/#CjQKILaqQTWUZks14mPRSn0m0zyU9A-buNMG6haQBmWrxJHeEhCc7HLIwCFZRNDw63MWj-fA

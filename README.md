# gurk 🥒
![CI][ci-badge] [![chat][chat-badge]][chat-link]

[Signal Messenger] client for terminal.

![screenshot](screenshot.png)

# Installation

## Pre-compiled binary

Download a pre-compiled binary from [Releases] for these targets :

* `x86-64 Linux GNU`
* `x86-64 Linux musl`
* `aarch64 Linux GNU`
* `aarch64 Linux musl` (>= `v0.2.4` only)
* `x86-64 Darwin`
* `aarch64 Darwin`

## From source (using `cargo`)

```shell
cargo install --git https://github.com/boxdot/gurk-rs gurk
```

## Arch Linux

Packaged in the AUR (either `bin` or `git`).

# Usage

Run

```
gurk
```

On the first run, it will open a QR code in your favorite image viewer, such that you can link the
client as a new device. This will also create a configuration file at the default [config
location][config-location]. For the configuration directives, see [`src/config.rs`].

Note: The binary cannot be published on crates.io, because it depends on several official Signal
libraries that are not available on crates.io.

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
* [X] Multiline messages; the `Enter` key sends the message.
* [ ] Viewing/sending of attachments (sending works).
* [ ] Support for blocked contacts/groups.
* [x] Reactions with emojis.
* [x] Open URL in selected message.

# Key bindings
* App navigation
  * `f1` Toggle help panel
  * `alt+tab` Switch between message input box and search bar
Message edition
  * `tab` Send emoji from input line as reaction on selected message.
  * `alt+enter` Add newline.
  * `ctrl+w` Delete last word
  * `enter` *when input box empty* Open URL from selected message
  * `enter` *otherwise* Send message
* Cursor
  * `alt+f / alt+Right / ctrl+Right` Move forward one word
  * `alt+b / alt+Left / ctrl+Left` Move backward one word
  * `ctrl+a / home` Move cursor to the beginning of the text
  * `ctrl+e / end` Move cursor the the end of the text
* Message/channel selection
  * `alt+Up / PgUp` Select previous message
  * `alt+Down / PgDown` Select next message
  * `ctrl+j / Up` Select previous channel
  * `ctrl+k / Down` Select next channel


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
[Releases]: https://github.com/boxdot/gurk-rs/releases

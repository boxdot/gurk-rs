# gurk 🥒
[![ci][ci-badge]][ci-link] [![chat][chat-badge]][chat-link]

[Signal Messenger] client for terminal.
pkg install net-im/gurk-rs
![screenshot](screenshot.png)

# Installation

## Pre-compiled binary

Download a pre-compiled binary from [Releases].

Or, if you have [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall):

```shell
cargo binstall --git https://github.com/boxdot/gurk-rs gurk
```

## From source (using `cargo`)

Prerequisites:

* [`protoc`] compiler
* `perl`

```shell
cargo install --git https://github.com/boxdot/gurk-rs gurk
```

## Arch Linux

```shell
pacman -S gurk
```
(as root)
- Official repository, tagged releases: [`gurk`](https://archlinux.org/packages/extra/x86_64/gurk)
```shell
yay -S gurk
```
- AUR source build from Git HEAD: [`gurk-git`](https://aur.archlinux.org/packages/gurk-bin)

## Nix/NixOS

Either per user:

```
$ nix-env --install gurk-rs
```

or system-wide:

```nix
environment.systemPackages = with pkgs; [ gurk-rs ];
```

# Freebsd
#### (as root)
```shell
pkg install net-im/gurk-rs
```
- pkg repositories


```shell
cd /usr/ports/net-im/gurk-rs
make install clean
```
- ports tree
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

# Key bindings

## Default keybindings

* App navigation
  * `f1` Toggle help panel.
  * `ctrl+c` Quit.
* Message input
  * `tab` Send emoji from input line as reaction on selected message.
  * `alt+enter` Switch between multi-line and singl-line input modes.
  * `alt+left`, `alt+right` Jump to previous/next word.
  * `ctrl+w / ctrl+backspace / alt+backspace` Delete last word.
  * `ctrl+u` Delete to the start of the line.
  * `enter` *when input box empty in single-line mode* Open URL from selected message.
  * `enter` *otherwise* Send message.
* Multi-line message input
  * `enter` New line
  * `ctrl+j / Up` Previous line
  * `ctrl+k / Down` Next line
* Cursor
  * `alt+f / alt+Right / ctrl+Right` Move forward one word.
  * `alt+b / alt+Left / ctrl+Left` Move backward one word.
  * `ctrl+a / Home` Move cursor to the beginning of the line.
  * `ctrl+e / End` Move cursor the the end of the line.
* Message/channel selection
  * `esc` Reset message selection or close channel selection popup.
  * `alt+Up / alt+k / PgUp` Select previous message.
  * `alt+Down / alt+j / PgDown` Select next message.
  * `ctrl+j / Up` Select previous channel.
  * `ctrl+k / Down` Select next channel.
  * `ctrl+p` Open / close channel selection popup.
* Clipboard
  * `alt+y` Copy selected message to clipboard.
* Help menu
  * `esc` Close help panel.
  * `ctrl+j / Up / PgUp` Previous line
  * `ctrl+k / Down / PgDown` Next line

## File Uploads
  * `file:///path/to/file` Upload File "file" at path "/path/to/"
  * `file://clip` Upload Content of Clipboard

## Configuration

Upon startup, `gurk` tries to load configuration from one of the default locations:

1. `$XDG_CONFIG_HOME/gurk/gurk.toml`
2. `$XDG_CONFIG_HOME/gurk.toml`
3. `$HOME/.config/gurk/gurk.toml`
4. `$HOME/.gurk.toml`

## Custom keybindings
The default keybindings can be overwritten at startup by configuring
keybindings in `gurk.toml` using the format `keybindings.<mode>.<keycombination> =
"<command>"`. Valid commands are `anywhere`, `normal`, `message_selected`,
`channel_modal`, `multiline`, and `help`. Valid key combination specifiers are e.g. `left,
alt-j, ctrl-f, backspace, pagedown`. The default keybindings can be disabled by
setting `default_keybindings = false`. An empty command removes an existing
binding if it exists in the given mode. Configuration troubleshooted by running
`RUST_LOG=gurk=trace,presage=trace,libsignal=trace gurk --verbose` and examining the resulting `gurk.log`.

### Supported commands
```
help
quit
toggle_channel_modal
toggle_multiline
react
scroll help up|down entry
move_text previous|next character|word|line
select_channel previous|next
select_channel_modal previous|next
select_message previous|next entry
kill_line
kill_whole_line
kill_backward_line
kill_word
copy_message selected
beginning_of_line
end_of_line
delete_character previous
edit_message
open_url
open_file
```

### Example configuration
```toml
default_keybindings = true

[keybindings.anywhere]
ctrl-c = ""
ctrl-q = "quit"

[keybindings.normal]
ctrl-j = ""
ctrl-k = "kill_line"
ctrl-n = "select_channel next"
ctrl-p = "select_channel previous"
alt-c = "toggle_channel_modal"
up = "select_message previous entry"
down = "select_message next entry"

[keybindings.channel_modal]
ctrl-j = ""
ctrl-k = ""
ctrl-n = "select_channel_modal next"
ctrl-p = "select_channel_modal previous"

[keybindings.message_selected]
alt-y = ""
alt-w = "copy_message selected"
ctrl-t = "react :thumbsup:"
ctrl-h = "react ❤️"
```

## License

 * GNU Affero General Public License v3 only ([AGPL-3.0-only](LICENSE-AGPL-3.0) or
   https://www.gnu.org/licenses/agpl-3.0.en.html)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this document by you, as defined in the AGPL-3.0-only license,
shall be licensed as above, without any additional terms or conditions.

[Signal Messenger]: https://signal.org
[`presage`]: https://github.com/whisperfish/presage
[`src/config.rs`]: https://github.com/boxdot/gurk-rs/blob/main/src/config.rs
[chat-badge]: https://img.shields.io/badge/chat-on%20signal-brightgreen?logo=signal
[ci-badge]: https://github.com/boxdot/gurk-rs/actions/workflows/ci.yaml/badge.svg
[ci-link]: https://github.com/boxdot/gurk-rs/actions/workflows/ci.yaml
[chat-link]: https://signal.group/#CjQKILaqQTWUZks14mPRSn0m0zyU9A-buNMG6haQBmWrxJHeEhCc7HLIwCFZRNDw63MWj-fA
[config-location]: https://docs.rs/dirs/3.0.2/dirs/fn.config_dir.html
[Releases]: https://github.com/boxdot/gurk-rs/releases
[`protoc`]: https://github.com/protocolbuffers/protobuf?tab=readme-ov-file#protobuf-compiler-installation

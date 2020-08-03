# gurk 🥒
![CI](https://github.com/boxdot/gurk-rs/workflows/CI/badge.svg)

[Signal Messenger] client for terminal.

![screenshot](screenshot.png)

## Usage

You need to download and install [`signal-cli`], such that it is found in your `PATH`.

1. Download and install `signal-cli`
2. Follow the instructions at https://github.com/AsamK/signal-cli#usage to register a new client.
3. Install gurk with `cargo install gurk`
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
works on Linux. We should evaluate if it is possible to switch to the [`libsignal-service-rs`]
crate.

## License

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT License ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this document by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[Signal Messenger]: https://signal.org
[`signal-cli`]: https://github.com/AsamK/signal-cli
[`libsignal-service-rs`]: https://github.com/Michael-F-Bryan/libsignal-service-rs
[`src/config.rs`]: https://github.com/boxdot/gurk-rs/blob/master/src/config.rs

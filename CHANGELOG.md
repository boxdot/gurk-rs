# Changelog

## 0.2.0 (Unreleased)

The highlight of this release is the usage of the native implementation of the Signal client
protocol via [presage]. This removes the dependency on [signal-cli] and makes `gurk` fully
standalone. For more defails, see [#41].

⚠️ This release has a breaking change of the data storage.

### Added

- Ctrl+J/K for channel up/down navigation ([#74])
- Added option to disable looping back when scrolling through messages. ([#72])
- Allow inter-word navigation with Alt/Ctrl+←→ ([#66])
- Handle reactions and show them as suffix of messages. ([#53])
- Keyboard shortcuts for word navigation ([#38])
- Scrolling messages ([#21])
- Mouse navigation of channels ([#24])
- New message notifications using notify-rust ([#19])

### Changed

- Change quoted reply-to text to a darker gray. ([#73])
- 🦀 Port to [presage]: native implementation of Signal client. ([#41])

### Fixed

- Fix init of data file by adding creation of default when none exists ([#48])
- Use local time zone when rendering time. ([#46])

[#19]: https://github.com/boxdot/gurk-rs/pull/19
[#24]: https://github.com/boxdot/gurk-rs/pull/24
[#21]: https://github.com/boxdot/gurk-rs/pull/21
[#38]: https://github.com/boxdot/gurk-rs/pull/38
[#41]: https://github.com/boxdot/gurk-rs/pull/41
[#46]: https://github.com/boxdot/gurk-rs/pull/46
[#48]: https://github.com/boxdot/gurk-rs/pull/48
[#53]: https://github.com/boxdot/gurk-rs/pull/53
[#66]: https://github.com/boxdot/gurk-rs/pull/66
[#72]: https://github.com/boxdot/gurk-rs/pull/72
[#73]: https://github.com/boxdot/gurk-rs/pull/73
[#74]: https://github.com/boxdot/gurk-rs/pull/74
[presage]: https://github.com/whisperfish/presage

## 0.1.1 (Oct 1, 2020)

### Added

- Fix cli linking to phone instruction link. ([#13])
- Respect `XDG_CONFIG_HOME` and `XDG_DATA_HOME`. ([#5])

### Fixed

- Invalid handling of empty channels list ([#7])

[#5]: https://github.com/boxdot/gurk-rs/pull/5
[#7]: https://github.com/boxdot/gurk-rs/pull/7
[#13]: https://github.com/boxdot/gurk-rs/pull/13

## 0.1.0 (Aug 2, 2020)

- Initial release based on [signal-cli]

[signal-cli]: https://github.com/AsamK/signal-cli

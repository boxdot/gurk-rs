# Changelog

## 0.4.0

### Added

- Copy selected message to clipboard ([#210])
- Implement storing and rendering of mentions ([#215], [#136])
- Sync contacts and groups from signal manager ([#226], [#227])

### Changed

- Replace search box by channel selection popup (Ctrl+p) ([#203])

### Fixed

- Do not create log file when logging is disabled ([#204])
- Fix blocking contacts sync ([#216])

## Internal

- replace tui with ratatui ([#238])

[#203]: https://github.com/boxdot/gurk-rs/pull/203
[#204]: https://github.com/boxdot/gurk-rs/pull/204
[#210]: https://github.com/boxdot/gurk-rs/pull/210
[#136]: https://github.com/boxdot/gurk-rs/pull/136
[#215]: https://github.com/boxdot/gurk-rs/pull/215
[#216]: https://github.com/boxdot/gurk-rs/pull/216
[#226]: https://github.com/boxdot/gurk-rs/pull/226
[#227]: https://github.com/boxdot/gurk-rs/pull/227
[#238]: https://github.com/boxdot/gurk-rs/pull/238

## 0.3.0

⚠️ This release requires relinking.

### Added

- Add notifications config toggling system notifications ([#188], [#192])

### Changed

- Upgrade presage (this will force relinking the device, due to incompatible changes) ([#182])

### Fixed

-  Use maintenance fixed branch of presage with updated root CA ([#189], [#190])

[#182]: https://github.com/boxdot/gurk-rs/pull/182
[#188]: https://github.com/boxdot/gurk-rs/pull/188
[#189]: https://github.com/boxdot/gurk-rs/pull/189
[#190]: https://github.com/boxdot/gurk-rs/pull/190
[#192]: https://github.com/boxdot/gurk-rs/pull/192

## 0.2.5

### Changed

- Replace log4rs with tracing ([#158], [#160])
- Display date only once per day ([#164], [#168])

### Fixed

- Fixed receiving direct messages sent from another device ([#162])
- Improve name resolution ([#167])
- Fix loosing incoming messages in groups ([#172])
- Increase chrono version for vulnerability fix ([#178])

[#158]: https://github.com/boxdot/gurk-rs/pull/158
[#160]: https://github.com/boxdot/gurk-rs/pull/160
[#162]: https://github.com/boxdot/gurk-rs/pull/162
[#164]: https://github.com/boxdot/gurk-rs/pull/164
[#167]: https://github.com/boxdot/gurk-rs/pull/167
[#168]: https://github.com/boxdot/gurk-rs/pull/168
[#172]: https://github.com/boxdot/gurk-rs/pull/172
[#178]: https://github.com/boxdot/gurk-rs/pull/178

## 0.2.4

### Added

- Add support for downloading attachments ([#122])
- Add release build for `aarch64-unknown-musl` ([#126])
- Show qrcode in terminal instead of PNG viewer ([#128])
- Document key bindings and packages ([#130])
- Sync contacts ([#146])
- Add visual aid (emoji) for stickers ([#148])

### Changed

- Add cursor tracking and multi-line input navigation ([#131])
- New visual style for receipts ([#135], [#142], [#144])

### Fixed

- Bug: infinite loop while skipping words on input box ([#129], [#131])
- Fix fail on contact sync for contacts without a UUID ([#152])
- Return upon unknown group ([#133])
- Fix: Notifications bump direct messages channel up ([#134])
- Fix fail on contact sync for contacts without a UUID ([#152])

[#122]: https://github.com/boxdot/gurk-rs/pull/122
[#126]: https://github.com/boxdot/gurk-rs/pull/126
[#128]: https://github.com/boxdot/gurk-rs/pull/128
[#129]: https://github.com/boxdot/gurk-rs/pull/129
[#130]: https://github.com/boxdot/gurk-rs/pull/130
[#131]: https://github.com/boxdot/gurk-rs/pull/131
[#133]: https://github.com/boxdot/gurk-rs/pull/133
[#134]: https://github.com/boxdot/gurk-rs/pull/134
[#135]: https://github.com/boxdot/gurk-rs/pull/135
[#142]: https://github.com/boxdot/gurk-rs/pull/142
[#144]: https://github.com/boxdot/gurk-rs/pull/144
[#146]: https://github.com/boxdot/gurk-rs/pull/146
[#148]: https://github.com/boxdot/gurk-rs/pull/148
[#152]: https://github.com/boxdot/gurk-rs/pull/152

## 0.2.3

### Added

- Add help panel ([#107])
- Basic multiline editing support ([#109])
- Add search bar + receipt notifications ([#114])

[#107]: https://github.com/boxdot/gurk-rs/pull/107
[#109]: https://github.com/boxdot/gurk-rs/pull/109
[#114]: https://github.com/boxdot/gurk-rs/pull/114

### Fixed

- Fix linking device ([#101], [#102])
- Fix and isolate message receipts ([#116])

[#101]: https://github.com/boxdot/gurk-rs/pull/101
[#102]: https://github.com/boxdot/gurk-rs/pull/102
[#116]: https://github.com/boxdot/gurk-rs/pull/116

## 0.2.2

### Added

- Add basic emojis and reactions support. ([#91])
- Open URL (if any) in selected message on Enter when input is empty. ([#99])
- Send attachments from file:// paths (#[100]).

### Fixed

[#91]: https://github.com/boxdot/gurk-rs/pull/91
[#99]: https://github.com/boxdot/gurk-rs/pull/99
[#100]: https://github.com/boxdot/gurk-rs/pull/100

## 0.2.1

### Fixed

- Fix formatting of phone number and update user name on start. ([#78])
- Fix an overflow error and crash by adding a subtraction check. ([#88])

[#78]: https://github.com/boxdot/gurk-rs/pull/78
[#88]: https://github.com/boxdot/gurk-rs/pull/88

## 0.2.0

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

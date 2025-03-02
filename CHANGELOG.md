# Changelog

## 0.6.3

### Added

- Store uploaded attachments in gurk's data directory (#375)
- Show deprecated config keys on start (#365, #377)

### Fixes

- Attachments opening handling (#371)
  - Remove invalid characters from attachment file names
  - Open attachment in selected message on Enter

### Internal

- Sqlite is the default data storage backend (#365, #377)
- Upgrade to edition 2024 (#372)

[#365]: https://github.com/boxdot/gurk-rs/pull/365
[#371]: https://github.com/boxdot/gurk-rs/pull/371
[#372]: https://github.com/boxdot/gurk-rs/pull/372
[#375]: https://github.com/boxdot/gurk-rs/pull/375
[#377]: https://github.com/boxdot/gurk-rs/pull/377

## 0.6.2

### Added

- Add command for opening file attachments (#356)
- Render qr code into a temporary PNG file (#364)

### Fixes

- Bump presage to fix linking (HTTP 409: Conflict) (#358)
- Security upgrade presage (#362)

[#356]: https://github.com/boxdot/gurk-rs/pull/356
[#358]: https://github.com/boxdot/gurk-rs/pull/358
[#362]: https://github.com/boxdot/gurk-rs/pull/362
[#364]: https://github.com/boxdot/gurk-rs/pull/364

## 0.6.1

### Fixes

- Don't relink implicitly when the manager cannot be loaded. (#345)
- Update presage fixing connection to Signal servers (#350)

## 0.6.0

### Added

- Render a message when there are no channels/messages ([#338])
- Keyboard bindings for emoji reactions ([#327], [#330])

### Fixes

- Random state reusage ([#326]):
  This fixes the long-stating issue [#234] about losing the Signal session and
  losing messages between linked clients.
- Replace reaction in UI instead of always removing it when replacing it ([#332])
- Ignore empty names from contact store ([#336])

### Internal

- Respect `RUST_LOG` when specifying --verbose ([#322])

[#234]: https://github.com/boxdot/gurk-rs/issues/234
[#326]: https://github.com/boxdot/gurk-rs/pull/326
[#332]: https://github.com/boxdot/gurk-rs/pull/332
[#336]: https://github.com/boxdot/gurk-rs/pull/336
[#338]: https://github.com/boxdot/gurk-rs/pull/338
[#330]: https://github.com/boxdot/gurk-rs/pull/330
[#327]: https://github.com/boxdot/gurk-rs/pull/327


## 0.5.2

### Added

- Add `colored_messages` config option ([#311])
- Handle read receipts from other clients ([#312])
- Add command, window mode, and keybinding logic ([#315], [#317])

### Fixes

- Process group messages without a profile key ([#318], [#319])
- Upgrade libsignal-client 0.51.1 -> 0.56.1 (fixes linking) ([#314])

[#311]: https://github.com/boxdot/gurk-rs/pull/311
[#312]: https://github.com/boxdot/gurk-rs/pull/312
[#314]: https://github.com/boxdot/gurk-rs/pull/314
[#315]: https://github.com/boxdot/gurk-rs/pull/315
[#317]: https://github.com/boxdot/gurk-rs/pull/317
[#318]: https://github.com/boxdot/gurk-rs/pull/318
[#319]: https://github.com/boxdot/gurk-rs/pull/319

## 0.5.1

### Added

- Edit messages ([#301])

### Fixes

- Fix unexpected response HTTP 409 during linking ([#299])

[#299]: https://github.com/boxdot/gurk-rs/pull/299
[#301]: https://github.com/boxdot/gurk-rs/pull/301

## 0.5.0

New configuration which enables encryption of the signal keystore and
the gurk messages database:

```
passphrase = "secret"
```

Previously unencrypted database is replaced by the encrypted one. Make
sure you backup your data before enabling this option.

After enabling encryption device has to be linked again.

### Added

- Key store and messages database encryption ([#283])

### Fixed

- Show self send attachments ([#278])
- Use profile names as user names ([#277])

### Internal

- Upgrade libsignal to v0.51.0 ([#294])
- Make sqlite the default storage ([#295])

[#277]: https://github.com/boxdot/gurk-rs/pull/277
[#278]: https://github.com/boxdot/gurk-rs/pull/278
[#283]: https://github.com/boxdot/gurk-rs/pull/283
[#294]: https://github.com/boxdot/gurk-rs/pull/294
[#295]: https://github.com/boxdot/gurk-rs/pull/295

## 0.4.3

Due to several fixes in `libsignal-service-rs`/`presage` and the upgrade of
libsignal protocol it is recommended to relink the account:

```
gurk --relink
```

### Added

- handle incoming edit messages ([#263])

### Fixed

- multiple instances unlink account ([#262])
- do not handle empty messages ([#265])
- cache contact names ([#268])
- upgrade libsignal protocol 0.32.0 -> 0.40.1 ([#269])

[#262]: https://github.com/boxdot/gurk-rs/pull/262
[#263]: https://github.com/boxdot/gurk-rs/pull/263
[#265]: https://github.com/boxdot/gurk-rs/pull/265
[#268]: https://github.com/boxdot/gurk-rs/pull/268
[#269]: https://github.com/boxdot/gurk-rs/pull/269

## 0.4.2

### Changed

- Utilize name offset space better ([#258])
- Store attachments under a shorter path ([#259])

### Fixed

- Duplicate key events on windows ([#249])
- Skipping sync group message from other device ([#251])
- Message linking ([#255])

### Internal

- Upgrade signal protocol to 0.32 ([#248])
- Upgrade sqlx ([#252])
- Add fibonacci backoff on reconnect ([#256])
- Reconnect websockets when those are closed ([#257])

[#248]: https://github.com/boxdot/gurk-rs/pull/248
[#249]: https://github.com/boxdot/gurk-rs/pull/249
[#251]: https://github.com/boxdot/gurk-rs/pull/251
[#252]: https://github.com/boxdot/gurk-rs/pull/252
[#255]: https://github.com/boxdot/gurk-rs/pull/255
[#256]: https://github.com/boxdot/gurk-rs/pull/256
[#257]: https://github.com/boxdot/gurk-rs/pull/257
[#258]: https://github.com/boxdot/gurk-rs/pull/258
[#259]: https://github.com/boxdot/gurk-rs/pull/259

## 0.4.1

### Added

- Add ephemeral status to sent messages on errors ([#222])
- Support bracketed paste ([#229])
- Add support for Ctrl+U to delete line backwards ([#230])
- Show attachment names or types ([#231])
- Add urgent bell support ([#233])
- Implement sending images directly from clipboard ([#232])
- Experimental impl of Storage via sqlite ([#225])
- Sync contacts and groups from signal manager ([#226], [#227])

### Internal

- replace tui with ratatui ([#238])

[#222]: https://github.com/boxdot/gurk-rs/pull/222
[#225]: https://github.com/boxdot/gurk-rs/pull/225
[#226]: https://github.com/boxdot/gurk-rs/pull/226
[#229]: https://github.com/boxdot/gurk-rs/pull/229
[#230]: https://github.com/boxdot/gurk-rs/pull/230
[#231]: https://github.com/boxdot/gurk-rs/pull/231
[#232]: https://github.com/boxdot/gurk-rs/pull/232
[#233]: https://github.com/boxdot/gurk-rs/pull/233
[#238]: https://github.com/boxdot/gurk-rs/pull/238

## 0.4.0

### Added

- Copy selected message to clipboard ([#210])
- Implement storing and rendering of mentions ([#215], [#136])

### Changed

- Replace search box by channel selection popup (Ctrl+p) ([#203])

### Fixed

- Do not create log file when logging is disabled ([#204])
- Fix blocking contacts sync ([#216])

[#136]: https://github.com/boxdot/gurk-rs/pull/136
[#203]: https://github.com/boxdot/gurk-rs/pull/203
[#204]: https://github.com/boxdot/gurk-rs/pull/204
[#210]: https://github.com/boxdot/gurk-rs/pull/210
[#215]: https://github.com/boxdot/gurk-rs/pull/215
[#216]: https://github.com/boxdot/gurk-rs/pull/216
[#227]: https://github.com/boxdot/gurk-rs/pull/227

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

# LG C1 HomeKit audit — 2026-10-28

Read-only inspection of the existing rooted TV. No pairing, settings, services,
firmware, capture or light-output changes. No debugger attached or functions
invoked inside the HomeKit process. Device addresses and pairing material are
excluded.

## Verified versions

| Item | Observed value | Evidence |
| --- | --- | --- |
| LG firmware | `03.53.45` | Bounded read of the firmware string used by the running HomeKit process's `webos_getFirmwareVersion` |
| OS | webOS TV `6.5.3` | `/etc/os-release` |
| Firmware build marker | `20250913080742` | `/etc/version`; not interpreted as a release date |
| Advertised HAP protocol | `1.1` | Bonjour `_hap._tcp` TXT `pv` |
| Accessory category | Television (`31`) | Bonjour TXT `ci` |
| Pairing status | Already paired | Bonjour TXT `sf=0` |
| Apple ADK version/build | `4.1` / `3fac0fb4` | Statically resolved return strings of `HAPGetVersion` / `HAPGetBuild` in the installed executable |
| Executable owner package | `lib32-airplay-agency`, `1.0.0-r2` | Installed opkg file list and control metadata |
| Embedded HomeKit source-build label | `lib32-homekit/1.0.0-58-r2` | Compiler source paths in the executable; not an independently installed package version |
| AirPlay package metadata | `9.24.0` | `/usr/palm/applications/airplay/package/packageinfo.json` |

Installed executable: `/usr/sbin/homekit` (1,727,996 bytes), SHA-256
`ce5999d7bdbaab044b90cb47e99080367e1e354df09ec49496f72da7988f7e7a`.

These numbers describe different layers. HAP `1.1` does not establish that the
implementation is obsolete. No model/region-specific LG update comparison or
current commercial Apple ADK/specification revision comparison was completed.

## Registered controls

The executable was copied to a local temporary directory for ELF inspection.
Its exported symbols and ARM Thumb instructions identified the service tables
and version strings. Bounded, aligned reads of the corresponding live process
structures confirmed both characteristic lists below, including their null
terminators. Reads were limited to service/characteristic pointer tables and the
firmware string; no pairing stores or keys were read.

| Live service | Registered characteristics, excluding the generic name entry |
| --- | --- |
| `televisionService` | Configured Name, Active (power), Active Identifier (input), Sleep Discovery Mode, Power Mode Selection, Remote Key |
| `speakerService` | Volume, Mute, Active, Volume Control Type, Volume Selector |

The executable includes input-source definitions and native input inventory,
application-launch and remote-key integration. Individual live input entries,
their names/visibility and supported key values were not enumerated.

Picture Mode, Brightness, Closed Captions, Current Media State and Display Order
have symbols/handlers in this executable but are absent from the live television
characteristic list. Their presence in the binary does **not** establish working
webOS integration. In particular, the inspected picture/brightness write
handlers update accessory state; this audit did not establish that they change
the actual picture settings.

This confirms server registration, not successful control through a paired
controller or appearance in Apple Home. No authenticated HAP `/accessories`
request, characteristic value reads, command writes or physical validation ran.
Speaker volume registration does not establish control of an attached AVR.

## What we could add

Candidates require native readback and later explicitly authorized write tests:

- App launch/selection, represented as HomeKit application inputs or a Matter
  Application Launcher. The installed binary already references webOS
  application launch and foreground-app APIs.
- Picture-mode selection through a custom HomeKit implementation, after mapping
  actual SDR/HDR/Dolby Vision modes and verifying native settings/readback.
  Merely adding LG's unused characteristic would not prove the backend works.
- Brightness, captions or playback state only where a genuine native getter and
  corresponding control exist. Display brightness, OLED pixel brightness and
  black level must not be conflated.
- Independent Hue/Nanoleaf sync controls as separate accessory functions,
  preserving their existing ownership and independence.

Prefer a separate, removable accessory process over patching LG's executable or
pairing storage. It would require its own pairing and persistent identity. It
could share an internal webOS control layer with a later Matter implementation;
neither implementation exists as a result of this audit. Standby availability
and wake-up still require separate validation.

## Matter 1.6.1 comparison

Evidence uses the official SDK's **`v1.6.1.0` tag**, not current master. The full
adopted specification PDFs were not inspected; SDK release notes alone do not
prove a feature's adopted/certifiable status.

| Capability | Existing LG registration | Matter model in the tagged SDK |
| --- | --- | --- |
| Power, input selection, remote keys | Present | On/Off, Media Input, Keypad Input |
| Volume/mute | Present | Keypad keys; a separate Audio Control definition is provisional |
| Seeking, playback position/speed, audio/subtitle track selection | No dedicated controls in inspected TV table | Optional Media Playback features |
| Content search and URL launching | No dedicated service in inspected TV table | Content Launcher |
| App launching | No dedicated launcher in inspected TV table | Application Launcher; backend/catalog mapping still required |
| Picture modes | Omitted despite binary handlers | No general TV Picture Mode cluster identified in the inspected media model |

Matter therefore offers a richer structured **media** interface than LG currently
registers. These capabilities are not all new in 1.6.1. It cannot make Netflix,
other TV apps or an HDMI source provide seeking, metadata or track control unless
we can obtain and control those functions through a real backend.

The SDK's Audio Control definition includes volume/mute, optional startup/user
volume settings and basic bass/mid/treble correction, but explicitly declares
`apiMaturity="provisional"` and an `in-progress` generation source. Do not promise
it as adopted Matter 1.6.1 interoperability or ordinary controller UI support.

Apple's current published Matter compatibility list omits TVs. A Matter TV is
therefore not a documented replacement for HomeKit TV support in Apple Home.
For Apple Home/Siri, prioritize a custom HomeKit TV implementation if additional
controls justify it. For another controller or our own app, evaluate Matter's
actual supported media clusters before choosing it. Both protocols could later
use the same native operations, but implementing both now is unnecessary.

## Primary references

- [Apple: Matter accessory categories supported by Home](https://support.apple.com/en-au/102135)
- [Apple: HomeKit picture modes](https://developer.apple.com/documentation/homekit/hmcharacteristicvaluepicturemode)
- [Apple: accessory development and commercial HAP specification access](https://developer.apple.com/apple-home/)
- [Matter SDK v1.6.1.0 release notes](https://github.com/project-chip/connectedhomeip/releases/tag/v1.6.1.0)
- [Tagged Media Playback model](https://github.com/project-chip/connectedhomeip/blob/v1.6.1.0/data_model/1.6.1/clusters/MediaPlayback.xml)
- [Tagged Media Input model](https://github.com/project-chip/connectedhomeip/blob/v1.6.1.0/data_model/1.6.1/clusters/MediaInput.xml)
- [Tagged Content Launcher model](https://github.com/project-chip/connectedhomeip/blob/v1.6.1.0/data_model/1.6.1/clusters/ContentLauncher.xml)
- [Tagged provisional Audio Control definition](https://github.com/project-chip/connectedhomeip/blob/v1.6.1.0/src/app/zap-templates/zcl/data-model/chip/audio-control-cluster.xml)

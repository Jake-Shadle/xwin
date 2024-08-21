<!-- markdownlint-disable MD022 MD024 MD032 -->

# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->
## [Unreleased] - ReleaseDate
## [0.6.5] - 2024-08-21
### Fixed
- [PR#137](https://github.com/Jake-Shadle/xwin/pull/137) fixes the fix introduced in [PR#136](https://github.com/Jake-Shadle/xwin/pull/136).

## [0.6.4] - 2024-08-21
### Fixed
- [PR#136](https://github.com/Jake-Shadle/xwin/pull/136) fixed an issue introduced in [PR#131](https://github.com/Jake-Shadle/xwin/pull/131) where symlink disabling when a case-insensitive file system was detected was...not being respected. At all.

## [0.6.3] - 2024-08-09
### Fixed
- [PR#134](https://github.com/Jake-Shadle/xwin/pull/134) added back onecoreuap headers that were moved from the main SDK header package in recent versions of the SDK. Thanks [@tomager](https://github.com/tomager)!

## [0.6.2] - 2024-07-02
### Fixed
- [PR#131](https://github.com/Jake-Shadle/xwin/pull/131) resolved [#130](https://github.com/Jake-Shadle/xwin/issues/130) by adding detection of case-insensitive file systems, which then disables symlink creation since it is not needed, and breaks.

## [0.6.1] - 2024-06-30
### Fixed
- [PR#129](https://github.com/Jake-Shadle/xwin/pull/129) fixed [#128](https://github.com/Jake-Shadle/xwin/issues/128) by adding the additional `onecoreuap` MSI package that contains headers that were previously (before SDK 10.0.26100) part of other MSI packages. Thanks [@bigfoodK](https://github.com/bigfoodK)!

## [0.6.0] - 2024-06-03
### Added
- [PR#123](https://github.com/Jake-Shadle/xwin/pull/123) (a rework of [#119](https://github.com/Jake-Shadle/xwin/pull/119)) adds the ability to splat in the format understood by clang-cl `/winsysroot` option.

## [0.5.2] - 2024-05-06
### Changed
- [PR#117](https://github.com/Jake-Shadle/xwin/pull/117) updated a few crates, notably `zip`.

## [0.5.1] - 2024-04-02
### Changed
- [PR#116](https://github.com/Jake-Shadle/xwin/pull/116) (a rework of [#115](https://github.com/Jake-Shadle/xwin/pull/115)) improves the speed of the `x86_64-unknown-linux-musl` binary by using `mimalloc`.

## [0.5.0] - 2023-11-13
### Changed
- [PR#110](https://github.com/Jake-Shadle/xwin/pull/110) changed how `Ctx` is built. It was getting too complicated to support niche use cases, some of which didn't belong in a library (like reading environment variables), so this functionality has been completely removed. Instead, one must pass in a `ureq::Agent` that is fully configured how the user wants it.
- [PR#110](https://github.com/Jake-Shadle/xwin/pull/110) changed the environment variable read to the `xwin` binary instead of the library, as well as its name `https_proxy` -> `HTTPS_PROXY`, and added it to an an option on the command line.

## [0.4.1] - 2023-11-09
### Fixed
- [PR#108](https://github.com/Jake-Shadle/xwin/pull/108) resolved [#107](https://github.com/Jake-Shadle/xwin/issues/107) by fixing the Window symlink code added in [PR#105](https://github.com/Jake-Shadle/xwin/pull/105) and only using it in the two cases it was needed.

## [0.4.0] - 2023-11-07
### Added
- [PR#101](https://github.com/Jake-Shadle/xwin/pull/101) resolved [#28](https://github.com/Jake-Shadle/xwin/issues/28), [#84](https://github.com/Jake-Shadle/xwin/issues/84), and [#85](https://github.com/Jake-Shadle/xwin/issues/85) by adding a `minimize` command that straces a cargo build to write a `map` file that can be used by a `splat` command to only splat the headers and libraries actually needed to build, drastically reducing the splat output (eg. 1.3GiB -> 101MiB). This `map` file also allows the creation of symlinks on a per-file basis, allowing users to create their own symlinks if needed.
- [PR#104](https://github.com/Jake-Shadle/xwin/pull/104) resolved [#103](https://github.com/Jake-Shadle/xwin/issues/103) by allowing custom certificates to be specified via the `SSL_CERT_FILE`, `CURL_CA_BUNDLE`, or `REQUESTS_CA_BUNDLE` environment variables. `xwin` must be compiled with the `native-tls` feature for this to function. Thanks [@Owen-CH-Leung](https://github.com/Owen-CH-Leung)!
- [PR#105](https://github.com/Jake-Shadle/xwin/pull/105) supplanted [#100](https://github.com/Jake-Shadle/xwin/pull/100), allowing creation of symlinks on a Windows host. Thanks [@sykhro](https://github.com/sykhro)!

## [0.3.1] - 2023-09-12
### Changed
- [PR#99](https://github.com/Jake-Shadle/xwin/pull/99) changed the default VS manifest version from 16 -> 17. You can preserve the old behavior by passing `--manifest-version 16` on the cmd line.

### Fixed
- [PR#99](https://github.com/Jake-Shadle/xwin/pull/99) resolved [#92](https://github.com/Jake-Shadle/xwin/issues/92) by only failing if matching relative paths didn't have the same contents. This currently only applies to one file, `appnotify.h`, which is present in the SDK headers and Store headers.

## [0.3.0] - 2023-09-12
### Changed
- [PR#93](https://github.com/Jake-Shadle/xwin/pull/93) added the ability to specify a download timeout for each individual download, and changed the default from infinite to 60 seconds, so that xwin will error if the remote HTTP server is slow/unresponsive. Thanks [@dragonmux](https://github.com/dragonmux)!

## [0.2.15] - 2023-09-11
### Changed
- [PR#93](https://github.com/Jake-Shadle/xwin/pull/93) added the ability to specify a download timeout for each individual download, and changed the default from infinite to 60 seconds, so that xwin will error if the remote HTTP server is slow/unresponsive. Thanks [@dragonmux](https://github.com/dragonmux)!

## [0.2.14] - 2023-06-20
### Fixed
- [PR#90](https://github.com/Jake-Shadle/xwin/pull/90) fixed a problem caused by [PR#87](https://github.com/Jake-Shadle/xwin/pull/87).

## [0.2.13] - 2023-06-15
### Changed
- [PR#88](https://github.com/Jake-Shadle/xwin/pull/88) updated dependencies.

### Added
- [PR#87](https://github.com/Jake-Shadle/xwin/pull/87) added binaries for `aarch64-unknown-linux-musl`

## [0.2.12] - 2023-03-31
### Fixed
- [PR#77](https://github.com/Jake-Shadle/xwin/pull/77) resolved [#76](https://github.com/Jake-Shadle/xwin/issues/76) by correctly handling the retrieval of the latest SDK version, regardless of whether it is for the Windows 10 or 11 SDK.

## [0.2.11] - 2023-03-06
### Fixed
- [PR#74](https://github.com/Jake-Shadle/xwin/pull/74) resolved [#70](https://github.com/Jake-Shadle/xwin/issues/70) by creating symlinks for SDK headers that are included by the CRT and ATL headers.
- [PR#74](https://github.com/Jake-Shadle/xwin/pull/74) fixed an issue where debug symbols were splatted to disk even when not requested.

## [0.2.10] - 2022-11-30
### Fixed
- [PR#67](https://github.com/Jake-Shadle/xwin/pull/67) fixed an issue where incorrect packages could be selected due to using string ordering on strings that could both be version strings and regular non-version strings. Thanks [@mite-user](https://github.com/mite-user)!

## [0.2.9] - 2022-10-14
### Added
- [PR#62](https://github.com/Jake-Shadle/xwin/pull/62) added release builds for Windows, closing [#58](https://github.com/Jake-Shadle/xwin/issues/58).

### Changed
- [PR#61](https://github.com/Jake-Shadle/xwin/pull/61) updated clap to 4.0. Thanks [@messense](https://github.com/messense)!

## [0.2.8] - 2022-09-07
### Added
- [PR#59](https://github.com/Jake-Shadle/xwin/pull/59) added support for installing the Active Template Library (ATL). Thanks [@pascalkuthe](https://github.com/pascalkuthe)!

## [0.2.7] - 2022-08-29
### Added
- No changes in xwin itself, but now prebuilt binaries for `apple-darwin` are supplied.

## [0.2.6] - 2022-08-26
### Changed
- Updated dependencies, notably `indicatif` and `insta`.

## [0.2.5] - 2022-06-21
### Changed
- [PR#52](https://github.com/Jake-Shadle/xwin/pull/52) updated dependencies, including openssl-src to fix various issues raised by Github security advisories.

## [0.2.4] - 2022-05-23
### Added
- [PR#50](https://github.com/Jake-Shadle/xwin/pull/50) added the ability to specify an HTTPS proxy via the `https_proxy` environment variable. Thanks [@j-raccoon](https://github.com/j-raccoon)!

## [0.2.3] - 2022-05-16
### Fixed
- [PR#48](https://github.com/Jake-Shadle/xwin/pull/48) fixed an issue introduced in [PR#47](https://github.com/Jake-Shadle/xwin/pull/47) when using multiple architectures. Thanks [@messense](https://github.com/messense)!

## [0.2.2] - 2022-05-16
### Changed
- [PR#45](https://github.com/Jake-Shadle/xwin/pull/45) replaced `reqwest` with `ureq` which significantly reduced dependencies. It also made `rustls` an optional (but default) TLS implementation in addition to supporting native TLS for arcane platforms that are not supported by `ring`. Thanks [@messense](https://github.com/messense)!
- [PR#46](https://github.com/Jake-Shadle/xwin/pull/46) updated MSI to 0.5. Thanks [@messense](https://github.com/messense)!

### Added
- [PR#47](https://github.com/Jake-Shadle/xwin/pull/47) added symlinks to support the usage of the `/vctoolsdir` and `/winsdkdir` options in `clang-cl`, which allow for a more concise compiler invocation. I would point you to official docs for this but apparently there are none. Thanks [@Qyriad](https://github.com/Qyriad)!

## [0.2.1] - 2022-05-04
### Changed
- [PR#41](https://github.com/Jake-Shadle/xwin/pull/41) added a symlink for `BaseTsd.h`. Thanks [@jemc](https://github.com/jemc)!
- [PR#42](https://github.com/Jake-Shadle/xwin/pull/42) updated dependencies, fixing a [CVE](https://rustsec.org/advisories/RUSTSEC-2022-0013).

## [0.2.0] - 2022-03-01
### Changed
- [PR#37](https://github.com/Jake-Shadle/xwin/pull/37) changed from structopt to clap v3 for arguments parsing. Thanks [@messense](https://github.com/messense)!
- [PR#38](https://github.com/Jake-Shadle/xwin/pull/38) fixed up the clap arguments to include metadata to be closer to the original structopt output with eg. `xwin -V`, however this exposed a problem that clap couldn't handle the old `--version <MANIFEST_VERSION>` flag since it clashed with `-V, --version`, so the flag has been renamed to `--manifest-version`. This is unfortunately a breaking change for the CLI.

## [0.1.10] - 2022-02-28
### Fixed
- [PR#34](https://github.com/Jake-Shadle/xwin/pull/34) changed some code so that it is possible to compile and run for `x86_64-pc-windows-msvc`, though this target is not explicitly support. Thanks [@messense](https://github.com/messense)!
- [PR#36](https://github.com/Jake-Shadle/xwin/pull/36) updated indicatif to `0.17.0-rc.6` and pinned it to fix [#35](https://github.com/Jake-Shadle/xwin/issues/35).

## [0.1.9] - 2022-02-28
### Fixed
- [PR#32](https://github.com/Jake-Shadle/xwin/pull/32) fixed the `--disable-symlinks` flag to _actually_ not emit symlinks, which is needed if the target filesystem is case-insensitive.

## [0.1.8] - 2022-02-28
### Fixed
- [PR#30](https://github.com/Jake-Shadle/xwin/pull/30) updated the indicatif pre-release as a workaround for `cargo install`'s [broken behavior](https://github.com/rust-lang/cargo/issues/7169). Thanks [@messense](https://github.com/messense)!

## [0.1.7] - 2022-02-24
### Fixed
- [PR#27](https://github.com/Jake-Shadle/xwin/pull/27) added a fixup for `Iphlpapi.lib => iphlpapi.lib`. Thanks [@jelmansouri](https://github.com/jelmansouri)!

## [0.1.6] - 2022-02-07
### Fixed
- [PR#22](https://github.com/Jake-Shadle/xwin/pull/22) added a fix for zeromq using a [mixed case include](https://github.com/zeromq/libzmq/blob/3070a4b2461ec64129062907d915ed665d2ac126/src/precompiled.hpp#L73). Thanks [@Jasper-Bekkers](https://github.com/Jasper-Bekkers)!
- [PR#23](https://github.com/Jake-Shadle/xwin/pull/23) updated dependencies, which included bumping `thread_local` to fix a [security advisory](https://rustsec.org/advisories/RUSTSEC-2022-0006).

## [0.1.5] - 2021-11-25
### Fixed
- [PR#19](https://github.com/Jake-Shadle/xwin/pull/19) resolved [#18](https://github.com/Jake-Shadle/xwin/issues/18) by removing a source of non-determinism in the output. It also made it so that some `Store` headers are no longer splatted to disk when targeting the `Desktop` variant alone.

## [0.1.4] - 2021-11-22
### Added
- [PR#17](https://github.com/Jake-Shadle/xwin/pull/17) resolved [#6](https://github.com/Jake-Shadle/xwin/issues/6) by adding the `--manifest` option so that users can specify an exact manifest to use rather than downloading the mutable one from the Microsoft CDN.

## [0.1.3] - 2021-11-17
### Fixed
- [PR#15](https://github.com/Jake-Shadle/xwin/pull/15) resolved [#14](https://github.com/Jake-Shadle/xwin/issues/14) by removing the unnecessary use of `tokio::main`. Thanks [@mite-user](https://github.com/mite-user)!
- [PR#13](https://github.com/Jake-Shadle/xwin/pull/13) resolved [#12](https://github.com/Jake-Shadle/xwin/issues/12) by using the actual output directory rather than a hardcoded default. Thanks [@mite-user](https://github.com/mite-user)!

## [0.1.2] - 2021-11-11
### Fixed
- [PR#11](https://github.com/Jake-Shadle/xwin/pull/11) added a workaround symlink for `Kernel32.lib` to fix the prevalent `time` crate in older versions. Thanks [@twistedfall](https://github.com/twistedfall)!

## [0.1.1] - 2021-08-24
### Fixed
- [PR#9](https://github.com/Jake-Shadle/xwin/pull/9) resolved [#8](https://github.com/Jake-Shadle/xwin/pull/9) by adding support for additional symlinks for each `.lib` in `SCREAMING` case, since [some crates](https://github.com/microsoft/windows-rs/blob/a27a74784ccf304ab362bf2416f5f44e98e5eecd/src/bindings.rs) link them that way.

## [0.1.0] - 2021-08-22
### Added
- Initial implementation if downloading, unpacking, and splatting of the CRT and Windows SDK. This first pass focused on targeting x86_64 Desktop, so targeting the Windows Store or other architectures is not guaranteed to work.

<!-- next-url -->
[Unreleased]: https://github.com/Jake-Shadle/xwin/compare/0.6.5...HEAD
[0.6.5]: https://github.com/Jake-Shadle/xwin/compare/0.6.4...0.6.5
[0.6.4]: https://github.com/Jake-Shadle/xwin/compare/0.6.3...0.6.4
[0.6.3]: https://github.com/Jake-Shadle/xwin/compare/0.6.2...0.6.3
[0.6.2]: https://github.com/Jake-Shadle/xwin/compare/0.6.1...0.6.2
[0.6.1]: https://github.com/Jake-Shadle/xwin/compare/0.6.0...0.6.1
[0.6.0]: https://github.com/Jake-Shadle/xwin/compare/0.5.2...0.6.0
[0.5.2]: https://github.com/Jake-Shadle/xwin/compare/0.5.1...0.5.2
[0.5.1]: https://github.com/Jake-Shadle/xwin/compare/0.5.0...0.5.1
[0.5.0]: https://github.com/Jake-Shadle/xwin/compare/0.4.1...0.5.0
[0.4.1]: https://github.com/Jake-Shadle/xwin/compare/0.4.0...0.4.1
[0.4.0]: https://github.com/Jake-Shadle/xwin/compare/0.3.1...0.4.0
[0.3.1]: https://github.com/Jake-Shadle/xwin/compare/0.3.0...0.3.1
[0.3.0]: https://github.com/Jake-Shadle/xwin/compare/0.2.15...0.3.0
[0.2.15]: https://github.com/Jake-Shadle/xwin/compare/0.2.14...0.2.15
[0.2.14]: https://github.com/Jake-Shadle/xwin/compare/0.2.13...0.2.14
[0.2.13]: https://github.com/Jake-Shadle/xwin/compare/0.2.12...0.2.13
[0.2.12]: https://github.com/Jake-Shadle/xwin/compare/0.2.11...0.2.12
[0.2.11]: https://github.com/Jake-Shadle/xwin/compare/0.2.10...0.2.11
[0.2.10]: https://github.com/Jake-Shadle/xwin/compare/0.2.9...0.2.10
[0.2.9]: https://github.com/Jake-Shadle/xwin/compare/0.2.8...0.2.9
[0.2.8]: https://github.com/Jake-Shadle/xwin/compare/0.2.7...0.2.8
[0.2.7]: https://github.com/Jake-Shadle/xwin/compare/0.2.6...0.2.7
[0.2.6]: https://github.com/Jake-Shadle/xwin/compare/0.2.5...0.2.6
[0.2.5]: https://github.com/Jake-Shadle/xwin/compare/0.2.4...0.2.5
[0.2.4]: https://github.com/Jake-Shadle/xwin/compare/0.2.3...0.2.4
[0.2.3]: https://github.com/Jake-Shadle/xwin/compare/0.2.2...0.2.3
[0.2.2]: https://github.com/Jake-Shadle/xwin/compare/0.2.1...0.2.2
[0.2.1]: https://github.com/Jake-Shadle/xwin/compare/0.2.0...0.2.1
[0.2.0]: https://github.com/Jake-Shadle/xwin/compare/0.1.10...0.2.0
[0.1.10]: https://github.com/Jake-Shadle/xwin/compare/0.1.9...0.1.10
[0.1.9]: https://github.com/Jake-Shadle/xwin/compare/0.1.8...0.1.9
[0.1.8]: https://github.com/Jake-Shadle/xwin/compare/0.1.7...0.1.8
[0.1.7]: https://github.com/Jake-Shadle/xwin/compare/0.1.6...0.1.7
[0.1.6]: https://github.com/Jake-Shadle/xwin/compare/0.1.5...0.1.6
[0.1.5]: https://github.com/Jake-Shadle/xwin/compare/0.1.4...0.1.5
[0.1.4]: https://github.com/Jake-Shadle/xwin/compare/xwin-0.1.3...0.1.4
[0.1.3]: https://github.com/Jake-Shadle/xwin/compare/xwin-0.1.2...xwin-0.1.3
[0.1.2]: https://github.com/Jake-Shadle/xwin/compare/xwin-0.1.1...xwin-0.1.2
[0.1.1]: https://github.com/Jake-Shadle/xwin/compare/0.1.0...xwin-0.1.1
[0.1.0]: https://github.com/Jake-Shadle/xwin/releases/tag/0.1.0

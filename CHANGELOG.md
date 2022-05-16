<!-- markdownlint-disable MD022 MD024 MD032 -->

# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->
## [Unreleased] - ReleaseDate
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
[Unreleased]: https://github.com/Jake-Shadle/xwin/compare/0.2.2...HEAD
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

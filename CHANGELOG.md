<!-- markdownlint-disable MD022 MD024 MD032 -->

# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->
## [Unreleased] - ReleaseDate
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
[Unreleased]: https://github.com/Jake-Shadle/xwin/compare/xwin-0.1.3...HEAD
[0.1.3]: https://github.com/Jake-Shadle/xwin/compare/xwin-0.1.2...xwin-0.1.3
[0.1.2]: https://github.com/Jake-Shadle/xwin/compare/xwin-0.1.1...xwin-0.1.2
[0.1.1]: https://github.com/Jake-Shadle/xwin/compare/0.1.0...xwin-0.1.1
[0.1.0]: https://github.com/Jake-Shadle/xwin/releases/tag/0.1.0

# xwin

[![Crates.io](https://img.shields.io/crates/v/xwin.svg)](https://crates.io/crates/xwin)
[![Docs](https://docs.rs/xwin/badge.svg)](https://docs.rs/xwin)
[![dependency status](https://deps.rs/repo/github/Jake-Shadle/xwin/status.svg)](https://deps.rs/repo/github/Jake-Shadle/xwin)
[![Build status](https://github.com/Jake-Shadle/xwin/workflows/CI/badge.svg)](https://github.com/Jake-Shadle/xwin/actions)

A utility for downloading and packaging the [Microsoft CRT](https://docs.microsoft.com/en-us/cpp/c-runtime-library/crt-library-features?redirectedfrom=MSDN&view=msvc-160) headers and libraries, and [Windows SDK](https://en.wikipedia.org/wiki/Microsoft_Windows_SDK) headers and libraries needed for compiling and linking programs targetting Windows.

## Introduction

The goal of this project is to create a root directory for both the CRT and Windows SDK that each contain all of the necessary includes and libraries needed for an application to compile and link from a non-Windows platform, using a native cross compiling toolchain like clang/LLVM.

### Thanks

Special thanks to <https://github.com/mstorsjo/msvc-wine> for the inspiration.

### License

This contribution is dual licensed under EITHER OF

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

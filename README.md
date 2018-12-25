hwndloop
========

[![CircleCI](https://circleci.com/gh/jmgao/hwndloop.svg?style=svg)](https://circleci.com/gh/jmgao/hwndloop) [![Docs](https://docs.rs/hwndloop/badge.svg)](https://docs.rs/hwndloop)

An implementation of an event loop backed by a Win32 window.

This is useful when interfacing with Windows APIs that communicate via messages
sent to windows (e.g. rawinput's [WM_INPUT_DEVICE_CHANGE](https://docs.microsoft.com/en-us/windows/desktop/inputdev/wm-input-device-change)).

WARNING: This crate is not yet API-stable. If you use this, expect to have to
follow API changes upon upgrading. Notably, things that currently panic will
probably switch to returning a Result at some point in the future.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

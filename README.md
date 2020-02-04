[![Crates.io](https://img.shields.io/crates/v/strawpoll.svg)](https://crates.io/crates/strawpoll)
[![Documentation](https://docs.rs/strawpoll/badge.svg)](https://docs.rs/strawpoll/)
[![Build Status](https://dev.azure.com/jonhoo/jonhoo/_apis/build/status/strawpoll?branchName=master)](https://dev.azure.com/jonhoo/jonhoo/_build/latest?definitionId=16&branchName=master)
[![Codecov](https://codecov.io/github/jonhoo/strawpoll/coverage.svg?branch=master)](https://codecov.io/gh/jonhoo/strawpoll)

Sometimes, you have a future that itself contains smaller futures. When
the larger future is polled, it polls those child futures to see if any
of them have made progress. This can be inefficient if polling such a
future is expensive; when the big future is woken up, it is usually
because _one_ of its child futures was notified, and ideally only that
one future should be polled. Polling the other child futures that were
_not_ notified is wasting precious cycles. This crate provides a wrapper
for `Future` types, and other types that you may wish to call
`poll`-like methods on that avoids such spurious calls to `poll`.

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

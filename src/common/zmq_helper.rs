//! ZMQ socket initialization helpers.
//!
//! Centralizes the HWM=0 policy mandated by `CLAUDE.md`:
//!
//! > 絶対にデータを落とさない。落とすくらいならシステムを止める。
//! > ZMQ ソケットは全て HWM=0（無制限バッファ）。
//!
//! Both `set_sndhwm(0)` (PUB) and `set_rcvhwm(0)` (SUB) disable the libzmq
//! ring-buffer cap. With HWM=0 a PUB will buffer in memory indefinitely
//! rather than dropping messages, and a SUB will keep accepting frames as
//! long as it can allocate. Combined with the unbounded tokio/crossbeam
//! channels downstream, this preserves the no-loss invariant across the
//! entire pipeline.
//!
//! # Why a helper?
//!
//! Pre-Phase-1 the code repeated `socket.get_socket().set_sndhwm(0)` (or
//! `set_rcvhwm(0)`) at 12 call sites. The duplication isn't dangerous on
//! its own, but it makes the policy easy to silently violate when adding a
//! new socket — there's no `grep`-able invariant that says "every PUB/SUB
//! goes through this one place". Routing through `apply_no_hwm` collapses
//! the surface to a single function whose docstring and call graph both
//! point at CLAUDE.md.
//!
//! Phase 3 (R-P8 ComponentRunner) will fold this helper into a trait
//! `default impl` so adding a new component cannot forget to call it.

use tmq::AsZmqSocket;

/// Disable the send high-water-mark on a PUB socket.
///
/// Returns the underlying `zmq::Result` so callers convert into their own
/// component-specific error type (`ReaderError`, `EmulatorError`,
/// `tmq::TmqError`, …).
#[inline]
pub fn pub_no_hwm<S: AsZmqSocket>(socket: &S) -> zmq::Result<()> {
    socket.get_socket().set_sndhwm(0)
}

/// Disable the receive high-water-mark on a SUB socket.
///
/// Returns the underlying `zmq::Result` so callers convert into their own
/// component-specific error type.
#[inline]
pub fn sub_no_hwm<S: AsZmqSocket>(socket: &S) -> zmq::Result<()> {
    socket.get_socket().set_rcvhwm(0)
}

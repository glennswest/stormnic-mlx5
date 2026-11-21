//! Make several things all or none (#22, finding 0368e57226f3): the DMA
//! buffers `Cmdq::new` and `Eth::open` allocate before the device sees any
//! of them. One that fails frees those made before it, so a bind that runs
//! out of boot-services memory leaves nothing behind for the next try.
//! No UEFI here, so `test/unwind.rs` checks it on the host.

/// Make `N` values with `make(ctx, i)`, in order. If one fails, the ones
/// made so far go to `undo(ctx, value)`, latest first, and the failure is
/// returned with its index.
pub fn all_or_none<C, T, E, const N: usize>(
    ctx: &mut C,
    mut make: impl FnMut(&mut C, usize) -> Result<T, E>,
    mut undo: impl FnMut(&mut C, T),
) -> Result<[T; N], (usize, E)> {
    let mut made: alloc::vec::Vec<T> = alloc::vec::Vec::with_capacity(N);
    for i in 0..N {
        match make(ctx, i) {
            Ok(v) => made.push(v),
            Err(e) => {
                while let Some(v) = made.pop() {
                    undo(ctx, v);
                }
                return Err((i, e));
            }
        }
    }
    match made.try_into() {
        Ok(a) => Ok(a),
        // N were pushed, so the conversion cannot fail.
        Err(_) => unreachable!(),
    }
}

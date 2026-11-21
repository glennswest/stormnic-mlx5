//! `src/unwind.rs`: buffers made all or none, so a failed allocation part
//! way frees the ones before it (#22, finding 0368e57226f3). Run by
//! `scripts/test-host.sh`.
extern crate alloc;
#[path = "../src/unwind.rs"]
mod unwind;

/// A pool that hands out numbered buffers and fails on request.
struct Pool {
    fail_at: Option<usize>,
    live: Vec<usize>,
    freed: Vec<usize>,
}

fn run<const N: usize>(fail_at: Option<usize>) -> (Result<[usize; N], (usize, &'static str)>, Pool) {
    let mut p = Pool { fail_at, live: Vec::new(), freed: Vec::new() };
    let r = unwind::all_or_none(
        &mut p,
        |p: &mut Pool, i| {
            if p.fail_at == Some(i) {
                return Err("out of pages");
            }
            p.live.push(i);
            Ok(i)
        },
        |p: &mut Pool, v| {
            p.live.retain(|&x| x != v);
            p.freed.push(v);
        },
    );
    (r, p)
}

#[test]
fn all_made_are_returned_in_order() {
    let (r, p) = run::<8>(None);
    assert_eq!(r, Ok([0, 1, 2, 3, 4, 5, 6, 7]));
    assert!(p.freed.is_empty());
}

#[test]
fn a_late_failure_frees_every_earlier_buffer() {
    // The eighth of Eth::open's buffers fails: the seven before it go back.
    let (r, p) = run::<8>(Some(7));
    assert_eq!(r, Err((7, "out of pages")));
    assert!(p.live.is_empty(), "leaked {:?}", p.live);
    assert_eq!(p.freed, [6, 5, 4, 3, 2, 1, 0]);
}

#[test]
fn every_failure_point_leaves_nothing() {
    // Cmdq::new's three, failing at each.
    for at in 0..3 {
        let (r, p) = run::<3>(Some(at));
        assert_eq!(r, Err((at, "out of pages")));
        assert!(p.live.is_empty());
        assert_eq!(p.freed.len(), at);
    }
}

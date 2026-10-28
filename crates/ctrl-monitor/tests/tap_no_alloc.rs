//! The producer side must never allocate: it runs in MIDI and audio
//! callbacks (docs/MONITOR.md §3.3).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ctrl_monitor::{tap, Clock, Dir, PortIdx, TapConfig};

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn pushing_short_long_and_dropped_packets_does_not_allocate() {
    let (mut tx, mut rx) = tap(TapConfig {
        slots: 64,
        bytes: 4096,
    });
    let clock = Clock::new();
    let long = [0x11u8; 1000];
    let before = ALLOCATIONS.with(Cell::get);
    for _ in 0..3 {
        tx.push(clock.now(), PortIdx(0), Dir::In, &[0x90, 60, 100]);
        tx.push(clock.now(), PortIdx(0), Dir::Out, &long);
    }
    // The byte ring (4096) takes one more; the rest are dropped, still
    // without allocating.
    for _ in 0..5 {
        tx.push(clock.now(), PortIdx(0), Dir::Out, &long);
    }
    let after = ALLOCATIONS.with(Cell::get);
    assert_eq!(after - before, 0, "the producer allocated");
    assert_eq!(tx.dropped(), 4);
    assert_eq!(rx.drain(|_| {}), 7);
}

#![feature(never_type)]

use std::{future::Future, pin::Pin, task::Poll, ptr};
use std::task::{Waker, RawWaker, RawWakerVTable, Context};

// See if we can run a basic `async fn`
pub async fn foo(x: &u32, y: u32) -> u32 {
    let y = &y;
    let z = 9;
    let z = &z;
    let y = async { *y + *z }.await;
    let a = 10;
    let a = &a;
    *x + y + *a
}

async fn add(x: u32, y: u32) -> u32 {
    let a = async { x + y };
    a.await
}

async fn build_aggregate(a: u32, b: u32, c: u32, d: u32) -> u32 {
    let x = (add(a, b).await, add(c, d).await);
    x.0 + x.1
}

enum Never {}
fn never() -> Never {
    panic!()
}

async fn includes_never(crash: bool, x: u32) -> u32 {
    let mut result = async { x * x }.await;
    if !crash {
        return result;
    }
    #[allow(unused)]
    let bad = never();
    result *= async { x + x }.await;
    drop(bad);
    result
}

async fn partial_init(x: u32) -> u32 {
    #[allow(unreachable_code)]
    let _x: (String, !) = (String::new(), return async { x + x }.await);
}

fn run_fut(mut fut: impl Future<Output=u32>, output: u32) {
    fn raw_waker_clone(_this: *const ()) -> RawWaker {
        panic!("unimplemented");
    }
    fn raw_waker_wake(_this: *const ()) {
        panic!("unimplemented");
    }
    fn raw_waker_wake_by_ref(_this: *const ()) {
        panic!("unimplemented");
    }
    fn raw_waker_drop(_this: *const ()) {}

    static RAW_WAKER: RawWakerVTable = RawWakerVTable::new(
        raw_waker_clone,
        raw_waker_wake,
        raw_waker_wake_by_ref,
        raw_waker_drop,
    );

    let waker = unsafe { Waker::from_raw(RawWaker::new(ptr::null(), &RAW_WAKER)) };
    let mut context = Context::from_waker(&waker);
    assert_eq!(unsafe { Pin::new_unchecked(&mut fut) }.poll(&mut context), Poll::Ready(output));
}

fn main() {
    let x = 5;
    run_fut(foo(&x, 7), 31);

    run_fut(build_aggregate(1, 2, 3, 4), 10);

    run_fut(includes_never(false, 4), 16);

    run_fut(partial_init(4), 8);
}

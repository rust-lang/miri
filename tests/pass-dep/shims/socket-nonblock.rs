//@ignore-target: linux
//@only-target: linux # Prevent test running at all when running `./miri test`
//@compile-flags: -Zmiri-disable-isolation

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn main() -> std::io::Result<()> {
    thread::spawn::<_, std::io::Result<()>>(|| {
        let listener = TcpListener::bind("[::1]:1234")?;
        loop {
            let (_, addr) = listener.accept()?;
            println!("received new IPv6 connection: {addr}")
        }
    });

    thread::spawn(|| {
        loop {
            // print a tick every second to show that
            // Miris main loop isn't blocked whilst waiting
            // for connections
            println!("tick");
            std::thread::sleep(Duration::from_millis(1000));
        }
    });

    let listener = TcpListener::bind("127.0.0.1:1234")?;
    loop {
        let (_, addr) = listener.accept()?;
        println!("received new IPv4 connection: {addr}")
    }
}

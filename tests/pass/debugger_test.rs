fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let x = 5;
    let y = 10;
    println!("before add");
    let z = add(x, y);
    println!("result: {}", z);
}

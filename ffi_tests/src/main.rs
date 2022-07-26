extern "C" {
        fn get_num(x: i32) -> i32;
        fn printer();
        fn get_dbl(x: i32) -> f64;
        fn test_stack_spill(a:i32, b:i32, c:i32, d:i32, e:i32, f:i32, g:i32, h:i32, i:i32, j:i32, k:i32, l:i32) -> i32;
        fn pointer_test() -> *mut i32;
        fn ptr_printer(x: *mut i32);
        fn ddref_print(x: *mut *mut i32);
}

//extern "C" { pub fn get_num () -> :: std :: os :: raw :: c_int ; }

fn main() {
        //let x;
        unsafe {
            let mut y: i32 = 45;
            let mut z = (&mut y) as *mut i32;
            let mut z = &mut z;
            println!("{:?}, {:?}, {:?}", z, *z, **z);
            println!("as **i32 in rust: {:?}", (z as *mut *mut i32));
             println!("as **i32 in rust: {:?}", **(z as *mut *mut i32));
            ddref_print(z as *mut *mut i32);
                /*
                    //println!("{}", get_num());
                    printer();
                    x = get_num(1);
//                    let y = get_dbl(x);
    
                    println!("{}", x);
                    printer();
                    let y = test_stack_spill(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12);
                    println!("{}", y);

                    let ptr = pointer_test();
                    
                    ptr_printer(ptr);
                    println!("In Rust this pointer has value: {:?}", *ptr);

                    *ptr = 10;
                    ptr_printer(ptr);
                    println!("In Rust this pointer has value: {:?}", *ptr);

                    //let ptr2 = pointer_test();
                    //println!("{:?}, {:?}", *ptr, *ptr2);*/
        }
        //println!("x: {:?}", x);
        //println!("rjeiworjweio");
}

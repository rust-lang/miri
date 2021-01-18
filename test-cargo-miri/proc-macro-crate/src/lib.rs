#[path = "../serde.rs"]
mod serde;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        super::serde::_it_works();
    }
}

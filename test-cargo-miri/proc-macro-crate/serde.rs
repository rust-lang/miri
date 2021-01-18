use serde::Deserialize;

#[derive(Deserialize)]
struct Data {
    _num: i32,
}

pub(crate) fn _it_works() {
    assert_eq!(serde_json::from_str::<Data>(r#"{"_num":-1}"#).unwrap()._num, -1);
}

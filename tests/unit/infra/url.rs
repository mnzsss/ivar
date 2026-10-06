use super::*;

#[test]
fn encodes_unreserved_characters_verbatim() {
    assert_eq!(encode_component("abc-123_.~", false), "abc-123_.~");
    assert_eq!(encode_component("abc-123_.~", true), "abc-123_.~");
}

#[test]
fn encodes_spaces_and_special_characters() {
    assert_eq!(encode_component("hello world", false), "hello%20world");
    assert_eq!(encode_component("hello world", true), "hello+world");
    assert_eq!(
        encode_component("a/b?c=d&e#f", false),
        "a%2Fb%3Fc%3Dd%26e%23f"
    );
}

use memcp::auto_store::content_hash;

#[test]
fn test_content_hash_deterministic() {
    let h1 = content_hash("hello world");
    let h2 = content_hash("hello world");
    let h3 = content_hash("different content");
    assert_eq!(h1, h2);
    assert_ne!(h1, h3);
}

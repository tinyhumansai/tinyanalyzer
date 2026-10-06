fn dump(n: tree_sitter::Node, src: &str, depth: usize, field: Option<&str>) {
    let text = if n.child_count()==0 { &src[n.byte_range()] } else { "" };
    println!("{}{}{} {:?} {}", "  ".repeat(depth), field.map(|f| format!("{f}: ")).unwrap_or_default(), n.kind(), n.is_named(), text);
    let mut c = n.walk();
    for (i, ch) in n.children(&mut c).enumerate() { dump(ch, src, depth+1, n.field_name_for_child(i as u32)); }
}
fn main() {
    let src = r#"#[cfg(test)]
mod t { #[test] fn a() { let x: Vec<u8> = vec![1, 2]; foo.bar("s", 'c', 1.5, true); if a { b } else if c { d } } }
impl<T> Foo for Bar<T> where T: Copy { fn g(&self, a: &str) -> Option<i32> { match a { "x" => Some(1), _ => None } } }
struct S { a: String, b: usize }
/// doc
fn h() { let c = |x| x + 1; }
"#;
    let mut p = tree_sitter::Parser::new();
    p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
    let t = p.parse(src, None).unwrap();
    dump(t.root_node(), src, 0, None);
}

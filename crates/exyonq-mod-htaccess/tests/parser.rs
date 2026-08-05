//! Parser golden tests.

use exyonq_mod_htaccess::parse_htaccess;

#[test]
fn comments_and_quoting() {
    let parsed = parse_htaccess(
        "t.htaccess",
        r#"
# comment
Redirect 301 "/old path" /new
"#,
    )
    .expect("parse");
    assert_eq!(parsed.directives[0].name, "redirect");
    assert_eq!(parsed.directives[0].args[1], "/old path");
}

#[test]
fn case_insensitive_directives() {
    let parsed = parse_htaccess("t.htaccess", "REWRITEENGINE On\n").expect("parse");
    assert_eq!(parsed.directives[0].name, "rewriteengine");
}

#[test]
fn rewrite_rule_parsed() {
    let parsed =
        parse_htaccess("t.htaccess", "RewriteRule ^/old$ /new [R=301,L]\n").expect("parse");
    assert_eq!(parsed.directives[0].name, "rewriterule");
}

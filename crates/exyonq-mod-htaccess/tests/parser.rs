/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
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

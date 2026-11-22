//! Golden role tests for the common profiles: the complete styled byte ranges of
//! each fixture, for one whole scan, for every two-chunk split and for one Unicode
//! scalar per turn.
use super::*;

const K: u16 = STYLE_KEYWORD;
const T: u16 = STYLE_TYPE;
const F: u16 = STYLE_FUNCTION;
const S: u16 = STYLE_STRING;
const N: u16 = STYLE_NUMBER;
const C: u16 = STYLE_COMMENT;

/// Styled `(start, end, role)` runs of `code` streamed in turns ending at `ends`;
/// the last turn is final. Each turn re-sends the previous turn's unconsumed suffix.
fn lex(language: Language, code: &str, ends: &[usize]) -> Vec<(usize, usize, u16)> {
    let mut state = State::default();
    let mut styles = Styles::new(0..code.len(), false);
    let mut at = 0;
    for &end in ends {
        styles.origin(at);
        let last = end == code.len();
        let consumed = stream(
            &code[at..end],
            language,
            &mut state,
            &mut styles,
            last,
            || false,
        )
        .expect("bounded lexical turn");
        assert!(code.is_char_boundary(at + consumed));
        at += consumed;
        assert!(end - at <= 128, "bounded ambiguous suffix: {}", end - at);
    }
    assert_eq!(at, code.len());
    styles
        .finish()
        .0
        .iter()
        .map(|span| (span.start as usize, span.end as usize, span.role))
        .collect()
}

fn named(code: &str, runs: &[(usize, usize, u16)]) -> Vec<String> {
    runs.iter()
        .map(|&(start, end, role)| {
            let name = match role {
                K => "keyword",
                T => "type",
                F => "function",
                S => "string",
                N => "number",
                C => "comment",
                _ => "other",
            };
            format!("{start}..{end} {:?} {name}", &code[start..end])
        })
        .collect()
}

/// Each `(token, role)` is the next styled run of `code` after the previous one,
/// and nothing else is styled.
#[track_caller]
fn golden(language: Language, code: &str, expected: &[(&str, u16)]) {
    let mut from = 0;
    let expected: Vec<_> = expected
        .iter()
        .map(|&(token, role)| {
            let start = from
                + code[from..]
                    .find(token)
                    .unwrap_or_else(|| panic!("{token:?} after byte {from}"));
            from = start + token.len();
            (start, from, role)
        })
        .collect();
    let whole = lex(language, code, &[code.len()]);
    assert_eq!(
        named(code, &whole),
        named(code, &expected),
        "{language:?} whole scan"
    );
    assert_eq!(whole, expected);
    for split in (1..code.len()).filter(|at| code.is_char_boundary(*at)) {
        assert_eq!(
            lex(language, code, &[split, code.len()]),
            whole,
            "{language:?} split at {split}"
        );
    }
    let scalars: Vec<_> = code
        .char_indices()
        .map(|(at, _)| at)
        .skip(1)
        .chain([code.len()])
        .collect();
    assert_eq!(
        lex(language, code, &scalars),
        whole,
        "{language:?} one scalar per turn"
    );
}

#[test]
fn cpp_preprocessor_types_functions_and_literals() {
    let code = "#include <vector>
#include \"local/config.h\"
#  define LIMIT(x) ((x) * 2u)
#if defined(_MSC_VER) && !defined(NDEBUG)
#pragma once
#endif
namespace geo::detail {
template <typename T, class... Rest>
struct Point final : Base {
    std::vector<std::string> names;
    std::size_t count = 0;
    static constexpr unsigned long long mask = 0xFFull;
    auto area() const noexcept -> double override;
    uint32_t id_t_value;
};
}
int main(int argc, char** argv) {
    std::cout << geo::detail::scale(argc, 1'000'000) << '\\n';
    co_return Widget::make<int>(u8\"text\", L'w', 1.5e-3f);
}
";
    golden(
        Language::Cpp,
        code,
        &[
            ("#include", K),
            ("<vector>", S),
            ("#include", K),
            ("\"local/config.h\"", S),
            ("#  define", K),
            ("LIMIT", F),
            ("2u", N),
            ("#if", K),
            ("defined", K),
            ("defined", K),
            ("#pragma", K),
            ("#endif", K),
            ("namespace", K),
            ("geo", T),
            ("detail", T),
            ("template", K),
            ("typename", K),
            ("T", T),
            ("class", K),
            ("Rest", T),
            ("struct", K),
            ("Point", T),
            ("final", K),
            ("std", T),
            ("vector", T),
            ("std", T),
            ("string", T),
            ("std", T),
            ("size_t", T),
            ("0", N),
            ("static", K),
            ("constexpr", K),
            ("unsigned", T),
            ("long", T),
            ("long", T),
            ("0xFFull", N),
            ("auto", T),
            ("area", F),
            ("const", K),
            ("noexcept", K),
            ("double", T),
            ("override", K),
            ("uint32_t", T),
            ("int", T),
            ("main", F),
            ("int", T),
            ("char", T),
            ("std", T),
            ("geo", T),
            ("detail", T),
            ("scale", F),
            ("1'000'000", N),
            ("'\\n'", S),
            ("co_return", K),
            ("Widget", T),
            ("int", T),
            ("u8\"text\"", S),
            ("L'w'", S),
            ("1.5e-3f", N),
        ],
    );
}

#[test]
fn cpp_declared_names_std_members_and_number_forms() {
    let code = "class /* doc */ Widget;
class DLL_EXPORT Gadget {};
enum class Mode : std::uint8_t { Fast };
using Callback = void (*)(int);
auto s = R\"sql(SELECT 1)sql\";
requires std::integral<T> && concept_check;
x = y and not z;
std::chrono::milliseconds wait{250ms};
throw std::runtime_error(\"bad\");
::global_call();
obj.method(0b1010'0101, 0x1.8p3, 017, 10ULL);
";
    golden(
        Language::Cpp,
        code,
        &[
            ("class", K),
            ("/* doc */", C),
            ("Widget", T),
            ("class", K),
            ("Gadget", T),
            ("enum", K),
            ("class", K),
            ("Mode", T),
            ("std", T),
            ("uint8_t", T),
            ("using", K),
            ("Callback", T),
            ("void", T),
            ("int", T),
            ("auto", T),
            ("R\"sql(SELECT 1)sql\"", S),
            ("requires", K),
            ("std", T),
            ("and", K),
            ("not", K),
            ("std", T),
            ("chrono", T),
            ("milliseconds", T),
            ("250ms", N),
            ("throw", K),
            ("std", T),
            ("runtime_error", T),
            ("\"bad\"", S),
            ("global_call", F),
            ("method", F),
            ("0b1010'0101", N),
            ("0x1.8p3", N),
            ("017", N),
            ("10ULL", N),
        ],
    );
}

#[test]
fn c_types_tags_and_preprocessor() {
    let code = "#include <stdio.h>
typedef struct node { struct node *next; size_t len; } node_t;
static _Bool ready(const FILE *f, int64_t n) { return n > 0x10UL && f; }
union value { long double d; unsigned char raw[16]; };
";
    golden(
        Language::C,
        code,
        &[
            ("#include", K),
            ("<stdio.h>", S),
            ("typedef", K),
            ("struct", K),
            ("node", T),
            ("struct", K),
            ("node", T),
            ("size_t", T),
            ("node_t", T),
            ("static", K),
            ("_Bool", T),
            ("ready", F),
            ("const", K),
            ("FILE", T),
            ("int64_t", T),
            ("return", K),
            ("0x10UL", N),
            ("union", K),
            ("value", T),
            ("long", T),
            ("double", T),
            ("unsigned", T),
            ("char", T),
            ("16", N),
        ],
    );
}

#[test]
fn rust_primitives_pascal_types_paths_macros_and_suffixes() {
    let code = "use std::collections::HashMap;
#[derive(Debug, Clone)]
pub struct Cache<'a, T> { map: HashMap<&'a str, Vec<T>>, hits: u64 }
impl<'a, T: Clone> Cache<'a, T> {
    pub fn get_or(&self, key: &str) -> Option<T> {
        let n = self.map.len().max(1_000usize) as f64 * 2.5e3_f64;
        println!(\"{n}\");
        let v: Vec<u8> = (0..n as usize).collect::<Vec<_>>().iter().map(|x| *x as u8).collect();
        async { other().await }; b\"bytes\"; 0xFF_u8;
        if a!=b { vec![1] } else { None }
    }
}
";
    golden(
        Language::Rust,
        code,
        &[
            ("use", K),
            ("std", T),
            ("collections", T),
            ("HashMap", T),
            ("derive", F),
            ("Debug", T),
            ("Clone", T),
            ("pub", K),
            ("struct", K),
            ("Cache", T),
            ("'a", T),
            ("T", T),
            ("HashMap", T),
            ("'a", T),
            ("str", T),
            ("Vec", T),
            ("T", T),
            ("u64", T),
            ("impl", K),
            ("'a", T),
            ("T", T),
            ("Clone", T),
            ("Cache", T),
            ("'a", T),
            ("T", T),
            ("pub", K),
            ("fn", K),
            ("get_or", F),
            ("self", K),
            ("str", T),
            ("Option", T),
            ("T", T),
            ("let", K),
            ("self", K),
            ("len", F),
            ("max", F),
            ("1_000usize", N),
            ("as", K),
            ("f64", T),
            ("2.5e3_f64", N),
            ("println!", F),
            ("\"{n}\"", S),
            ("let", K),
            ("Vec", T),
            ("u8", T),
            ("0", N),
            ("as", K),
            ("usize", T),
            ("collect", F),
            ("Vec", T),
            ("iter", F),
            ("map", F),
            ("as", K),
            ("u8", T),
            ("collect", F),
            ("async", K),
            ("other", F),
            ("await", K),
            ("b\"bytes\"", S),
            ("0xFF_u8", N),
            ("if", K),
            ("vec!", F),
            ("1", N),
            ("else", K),
            ("None", T),
        ],
    );
}

#[test]
fn python_decorators_definitions_builtins_and_prefixed_strings() {
    let code = "@dataclass(frozen=True)
@app.route(\"/items\")
class Item(BaseModel):
    def total(self, count: int = 0x1F, rate=1_000.5j) -> list[str]:
        return [str(x) for x in range(count)] + rb\"raw\" + f\"{self.name}\"
print(Item.from_dict({}), len(data), data.items(), None)
y = a @ b
";
    golden(
        Language::Python,
        code,
        &[
            ("@dataclass", F),
            ("True", K),
            ("@app.route", F),
            ("\"/items\"", S),
            ("class", K),
            ("Item", T),
            ("BaseModel", T),
            ("def", K),
            ("total", F),
            ("self", K),
            ("int", T),
            ("0x1F", N),
            ("1_000.5j", N),
            ("list", T),
            ("str", T),
            ("return", K),
            ("str", T),
            ("for", K),
            ("in", K),
            ("range", F),
            ("rb\"raw\"", S),
            ("f\"{self.name}\"", S),
            ("print", F),
            ("Item", T),
            ("from_dict", F),
            ("len", F),
            ("items", F),
            ("None", K),
        ],
    );
}

#[test]
fn java_types_annotations_members_and_suffixes() {
    let code = "@Override
public final class Box<T> extends AbstractList<T> implements Serializable {
    private static final long MAX = 0x7fff_ffffL;
    public Box(int size) { this.items = new ArrayList<>(size); }
    var list = List.of(1.5f, 2d); String.valueOf(Box.class);
}
";
    golden(
        Language::Java,
        code,
        &[
            ("Override", T),
            ("public", K),
            ("final", K),
            ("class", K),
            ("Box", T),
            ("T", T),
            ("extends", K),
            ("AbstractList", T),
            ("T", T),
            ("implements", K),
            ("Serializable", T),
            ("private", K),
            ("static", K),
            ("final", K),
            ("long", T),
            ("0x7fff_ffffL", N),
            ("public", K),
            ("Box", T),
            ("int", T),
            ("this", K),
            ("new", K),
            ("ArrayList", T),
            ("var", K),
            ("List", T),
            ("of", F),
            ("1.5f", N),
            ("2d", N),
            ("String", T),
            ("valueOf", F),
            ("Box", T),
            ("class", K),
        ],
    );
}

#[test]
fn csharp_directives_namespaces_declarations_and_calls() {
    let code = "#region Setup
using System.Collections.Generic;
namespace App.Core;
public record Person(string Name, int Age);
var items = new List<int>(); items.Add(42m.GetHashCode());
if (obj is Person p && obj as object != null) Console.WriteLine($\"{p.Name}\");
#endregion
";
    golden(
        Language::CSharp,
        code,
        &[
            ("#region", K),
            ("using", K),
            ("System", T),
            ("Collections", T),
            ("Generic", T),
            ("namespace", K),
            ("App", T),
            ("Core", T),
            ("public", K),
            ("record", K),
            ("Person", T),
            ("string", T),
            ("int", T),
            ("var", K),
            ("new", K),
            ("List", T),
            ("int", T),
            ("Add", F),
            ("42m", N),
            ("GetHashCode", F),
            ("if", K),
            ("is", K),
            ("Person", T),
            ("as", K),
            ("object", T),
            ("null", K),
            ("WriteLine", F),
            ("\"{p.Name}\"", S),
            ("#endregion", K),
        ],
    );
}

#[test]
fn go_builtin_types_declarations_and_calls() {
    let code = "package main
import \"fmt\"
type Point struct { X, Y float64 }
func (p *Point) Scale(k int) error { return fmt.Errorf(\"%d\", len(p.tags), 0x1p-2, 1_000i) }
var c chan<- rune = make(chan rune); _ = iota
";
    golden(
        Language::Go,
        code,
        &[
            ("package", K),
            ("import", K),
            ("\"fmt\"", S),
            ("type", K),
            ("Point", T),
            ("struct", K),
            ("float64", T),
            ("func", K),
            ("Scale", F),
            ("int", T),
            ("error", T),
            ("return", K),
            ("Errorf", F),
            ("\"%d\"", S),
            ("len", F),
            ("0x1p-2", N),
            ("1_000i", N),
            ("var", K),
            ("chan", K),
            ("rune", T),
            ("make", F),
            ("chan", K),
            ("rune", T),
            ("iota", K),
        ],
    );
}

#[test]
fn swift_directives_types_and_declarations() {
    let code = "#if os(iOS)
import UIKit
#endif
struct Card: Codable { let id: Int; var tags: [String] = [] }
extension Card { func render(at x: Double) async throws -> some View { Text(\"\\(id)\") } }
let v = Card.self; let n = 0x1.8p1 + 1_000.0
";
    golden(
        Language::Swift,
        code,
        &[
            ("#if", K),
            ("os", F),
            ("import", K),
            ("UIKit", T),
            ("#endif", K),
            ("struct", K),
            ("Card", T),
            ("Codable", T),
            ("let", K),
            ("Int", T),
            ("var", K),
            ("String", T),
            ("extension", K),
            ("Card", T),
            ("func", K),
            ("render", F),
            ("Double", T),
            ("async", K),
            ("throws", K),
            ("some", K),
            ("View", T),
            ("Text", T),
            ("\"\\(id)\"", S),
            ("let", K),
            ("Card", T),
            ("self", K),
            ("let", K),
            ("0x1.8p1", N),
            ("1_000.0", N),
        ],
    );
}

#[test]
fn kotlin_classes_functions_and_suffixes() {
    let code = "data class User(val name: String, val age: Int = 0)
fun main() { val u = User(\"a\", 1L); println(u.copy(age = 2).toString()); listOf(1u, 0xFFuL) }
object Registry : Base()
";
    golden(
        Language::Kotlin,
        code,
        &[
            ("data", K),
            ("class", K),
            ("User", T),
            ("val", K),
            ("String", T),
            ("val", K),
            ("Int", T),
            ("0", N),
            ("fun", K),
            ("main", F),
            ("val", K),
            ("User", T),
            ("\"a\"", S),
            ("1L", N),
            ("println", F),
            ("copy", F),
            ("2", N),
            ("toString", F),
            ("listOf", F),
            ("1u", N),
            ("0xFFuL", N),
            ("object", K),
            ("Registry", T),
            ("Base", T),
        ],
    );
}

#[test]
fn javascript_classes_calls_and_member_names() {
    let code = "import { readFile } from \"node:fs\";
export default class Store extends EventTarget {
  async load(path) { const data = await readFile(path); return JSON.parse(data).items ?? 10n; }
}
promise.then(done).catch(fail).finally(() => new Map().delete(key));
function helper() {}
";
    golden(
        Language::JavaScript,
        code,
        &[
            ("import", K),
            ("\"node:fs\"", S),
            ("export", K),
            ("default", K),
            ("class", K),
            ("Store", T),
            ("extends", K),
            ("EventTarget", T),
            ("async", K),
            ("load", F),
            ("const", K),
            ("await", K),
            ("readFile", F),
            ("return", K),
            ("parse", F),
            ("10n", N),
            ("then", F),
            ("catch", F),
            ("finally", F),
            ("new", K),
            ("Map", T),
            ("delete", F),
            ("function", K),
            ("helper", F),
        ],
    );
}

#[test]
fn typescript_type_names_and_declarations() {
    let code = "interface Shape { area(): number; tags?: string[] }
type Id = string | bigint;
enum Color { Red }
abstract class Square implements Shape { constructor(private side: number) {} area(): number { return this.side ** 2; } }
const s = shape as unknown as Square; let v: void; node.type;
";
    golden(
        Language::TypeScript,
        code,
        &[
            ("interface", K),
            ("Shape", T),
            ("area", F),
            ("number", T),
            ("string", T),
            ("type", K),
            ("Id", T),
            ("string", T),
            ("bigint", T),
            ("enum", K),
            ("Color", T),
            ("Red", T),
            ("abstract", K),
            ("class", K),
            ("Square", T),
            ("implements", K),
            ("Shape", T),
            ("constructor", K),
            ("private", K),
            ("number", T),
            ("area", F),
            ("number", T),
            ("return", K),
            ("this", K),
            ("2", N),
            ("const", K),
            ("as", K),
            ("unknown", T),
            ("as", K),
            ("Square", T),
            ("let", K),
            ("void", T),
        ],
    );
}

#[test]
fn ruby_classes_methods_and_calls() {
    let code = "class Account < Base
  def deposit(amount) = @balance += amount
  def self.open; Account.new(0x10); end
end
";
    golden(
        Language::Ruby,
        code,
        &[
            ("class", K),
            ("Account", T),
            ("Base", T),
            ("def", K),
            ("deposit", F),
            ("def", K),
            ("self", K),
            ("Account", T),
            ("new", F),
            ("0x10", N),
            ("end", K),
            ("end", K),
        ],
    );
}

#[test]
fn shell_functions_builtins_and_expansions() {
    let code = "#!/bin/sh
deploy() { local target=\"$1\"; echo \"${target}\" $HOME ${#items[@]} ${name} $# $?; }
function cleanup { rm -rf \"$tmp\"; return 0; }
export PATH; readonly N=10
";
    golden(
        Language::Shell,
        code,
        &[
            ("#!/bin/sh", C),
            ("deploy", F),
            ("local", K),
            ("\"$1\"", S),
            ("\"${target}\"", S),
            ("$HOME", T),
            ("${#items", T),
            ("${name}", T),
            ("$#", T),
            ("$?", T),
            ("function", K),
            ("cleanup", F),
            ("\"$tmp\"", S),
            ("return", K),
            ("0", N),
            ("export", K),
            ("readonly", K),
            ("10", N),
        ],
    );
}

#[test]
fn sql_types_and_function_calls() {
    let code = "CREATE TABLE items (id INTEGER, name VARCHAR(40), price DECIMAL(10, 2));
SELECT COUNT(*), max(price) FROM items;
";
    golden(
        Language::Sql,
        code,
        &[
            ("CREATE", K),
            ("TABLE", K),
            ("INTEGER", T),
            ("VARCHAR", T),
            ("40", N),
            ("DECIMAL", T),
            ("10", N),
            ("2", N),
            ("SELECT", K),
            ("COUNT", F),
            ("max", F),
            ("FROM", K),
        ],
    );
}

/// Names longer than the classification bound stay plain, whatever follows them,
/// so a chunk boundary inside one cannot change its role.
#[test]
fn long_names_stay_plain_and_bounded_names_wait_for_their_next_byte() {
    let long = "n".repeat(65);
    let code = format!("int {long}(void); int named_{}(void);\n", "m".repeat(58));
    let short = format!("named_{}", "m".repeat(58));
    golden(
        Language::Cpp,
        &code,
        &[
            ("int", T),
            ("void", T),
            ("int", T),
            (short.as_str(), F),
            ("void", T),
        ],
    );
    // A name at the end of a turn waits for its next byte: `(`, `::` or `!` decide.
    for (language, text, held) in [
        (Language::Cpp, "x = compute", "compute"),
        (Language::Cpp, "auto v = std:", "std:"),
        (Language::Rust, "let v = items.collect::", "collect::"),
        (Language::Rust, "println!", "println!"),
    ] {
        let mut state = State::default();
        let mut styles = Styles::new(0..text.len(), false);
        let consumed = stream(text, language, &mut state, &mut styles, false, || false).unwrap();
        assert_eq!(&text[consumed..], held, "{language:?} {text:?}");
    }
}

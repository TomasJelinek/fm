//! Fixed per-language word tables: keywords, built-in type names, the declaration
//! words whose next name is a type or a function, and preprocessor directives.
//! Each table is sorted once and binary-searched per token; file content never
//! adds a word.
use super::{Expect, Language};
use midnight_plugin_sdk::api::viewer::{STYLE_KEYWORD, STYLE_TYPE};
use std::sync::OnceLock;

/// A listed word's role and what it makes of the next name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Word {
    pub role: u16,
    pub expect: Expect,
}

/// Names longer than this many bytes are never classified. A shorter name at the
/// end of a chunk stays with the caller until its following bytes arrive.
pub(super) const NAME_BYTES: usize = 64;

fn keywords(language: Language) -> &'static str {
    match language {
        Language::Rust => {
            "abstract as async await become box break const continue crate do dyn else enum extern false final fn for if impl in let loop macro match mod move mut override priv pub ref return self Self static struct super trait true try type typeof unsafe unsized use virtual where while yield"
        }
        Language::C => {
            "alignas alignof asm break case const constexpr continue default defined do else enum extern false for goto if inline nullptr register restrict return sizeof static static_assert struct switch thread_local true typedef typeof typeof_unqual union volatile while _Alignas _Alignof _Atomic _Generic _Noreturn _Pragma _Static_assert _Thread_local __attribute__ __declspec"
        }
        Language::Cpp => {
            "alignas alignof and and_eq asm bitand bitor break case catch class compl concept const consteval constexpr constinit const_cast continue co_await co_return co_yield decltype default defined delete do dynamic_cast else enum explicit export extern false final for friend goto if inline mutable namespace new noexcept not not_eq nullptr operator or or_eq override private protected public register reinterpret_cast requires return sizeof static static_assert static_cast struct switch template this thread_local throw true try typedef typeid typename union using virtual volatile while xor xor_eq _Pragma __attribute__ __declspec"
        }
        Language::JavaScript => {
            "async await break case catch class const continue debugger default delete do else export extends false finally for function if import in instanceof let new null of return static super switch this throw true try typeof undefined var void while with yield"
        }
        Language::TypeScript => {
            "abstract accessor as asserts async await break case catch class const constructor continue declare default delete do else enum export extends false finally for from function if implements import in infer instanceof interface is keyof let module namespace new null of override private protected public readonly return satisfies static super switch this throw true try type typeof undefined unique var while yield"
        }
        Language::Python => {
            "False None True and as assert async await break class cls continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return self try while with yield"
        }
        Language::Shell => {
            "break case continue coproc declare do done elif else esac eval exec exit export fi for function if in local readonly return select shift source then time trap typeset unset until while"
        }
        Language::Batch => {
            "assoc break call cd chdir cls color copy date del dir do echo else endlocal equ errorlevel exist exit for ftype geq goto gtr if in leq lss md mkdir move neq not path pause popd prompt pushd rd rem ren rename rmdir set setlocal shift start time title type ver verify vol"
        }
        Language::Json => "false null true",
        Language::Toml => "false true inf nan",
        Language::Java => {
            "abstract assert break case catch class const continue default do else enum extends final finally for goto if implements import instanceof interface native new package permits private protected public record return sealed static strictfp super switch synchronized this throw throws transient try var volatile when while yield true false null"
        }
        Language::CSharp => {
            "abstract and as async await base break case catch checked class const continue default delegate do else enum event explicit extern false finally fixed for foreach get global goto if implicit in init interface internal is lock nameof namespace new not null operator or out override params partial private protected public readonly record ref required return sealed set sizeof stackalloc static struct switch this throw true try typeof unchecked unsafe using var virtual volatile when where while with yield"
        }
        Language::Go => {
            "break case chan const continue default defer else fallthrough for func go goto if import interface iota map package range return select struct switch type var true false nil"
        }
        Language::Swift => {
            "actor any as associatedtype async await break case catch class continue convenience default defer deinit didSet do dynamic else enum extension fallthrough false fileprivate final for func get guard if import in indirect init inout internal is lazy let mutating nil nonisolated nonmutating open operator override package private protocol public repeat required rethrows return self Self set some static struct subscript super switch throw throws true try typealias unowned var weak where while willSet"
        }
        Language::Kotlin => {
            "as break class continue do else false for fun if in interface is null object package return super this throw true try typealias typeof val var when while by catch constructor delegate dynamic field file finally get import init param property receiver set setparam where actual abstract annotation companion const crossinline data enum expect external final infix inline inner internal lateinit noinline open operator out override private protected public reified sealed suspend tailrec vararg"
        }
        Language::Ruby => {
            "BEGIN END alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield"
        }
        // Case-insensitive languages list lowercase words.
        Language::Sql => {
            "select from where as join left right inner outer on group by order having limit offset insert into values update set delete create table drop alter index view distinct union all null true false and or not is like in exists case when then else end begin commit rollback"
        }
        Language::Yaml => "true false null True False Null TRUE FALSE NULL yes no on off",
        Language::Xml
        | Language::Lua
        | Language::Usda
        | Language::Css
        | Language::Makefile
        | Language::Csv
        | Language::CsvSemicolon
        | Language::Tsv => "",
    }
}

/// Built-in and standard type names that need no declaration context.
fn types(language: Language) -> &'static str {
    match language {
        Language::Rust => {
            "bool char f32 f64 i8 i16 i32 i64 i128 isize str u8 u16 u32 u64 u128 usize"
        }
        // C23 spells `auto` as a type, and `bool` without a header.
        Language::C => {
            "auto bool char double float int long short signed unsigned void FILE jmp_buf va_list _BitInt _Bool _Complex _Decimal128 _Decimal32 _Decimal64 _Imaginary"
        }
        Language::Cpp => {
            "auto bool char char8_t char16_t char32_t double float int long short signed unsigned void wchar_t FILE jmp_buf va_list __int8 __int16 __int32 __int64 __int128"
        }
        Language::TypeScript => "any bigint boolean never number object string symbol unknown void",
        Language::Python => {
            "bool bytearray bytes complex dict float frozenset int list memoryview object set str tuple"
        }
        Language::Java => "boolean byte char double float int long short void",
        Language::CSharp => {
            "bool byte char decimal double dynamic float int long nint nuint object sbyte short string uint ulong ushort void"
        }
        Language::Go => {
            "any bool byte comparable complex64 complex128 error float32 float64 int int8 int16 int32 int64 rune string uint uint8 uint16 uint32 uint64 uintptr"
        }
        Language::Sql => {
            "bigint binary bit blob boolean char character clob date datetime decimal double float int integer interval json jsonb money nchar numeric nvarchar real serial smallint text time timestamp tinyint uuid varbinary varchar xml"
        }
        _ => "",
    }
}

/// Keywords whose next name is declared or used as a type.
fn type_words(language: Language) -> &'static str {
    match language {
        Language::Rust => "enum impl mod struct trait type",
        Language::C => "enum struct union",
        Language::Cpp => "class concept enum namespace new struct typename union using",
        Language::JavaScript => "class extends new",
        Language::TypeScript => "class enum extends implements interface module namespace new type",
        Language::Python => "class",
        Language::Java => "class enum extends implements interface new record throws",
        Language::CSharp => "as class enum interface is namespace new record struct using",
        Language::Go => "type",
        Language::Swift => "actor associatedtype class enum extension protocol struct typealias",
        Language::Kotlin => "class interface object typealias",
        Language::Ruby => "class module",
        _ => "",
    }
}

/// Keywords whose next name is a declared function.
fn function_words(language: Language) -> &'static str {
    match language {
        Language::Rust => "fn",
        Language::Python | Language::Ruby => "def",
        Language::Go | Language::Swift => "func",
        Language::Kotlin => "fun",
        Language::JavaScript | Language::TypeScript | Language::Shell => "function",
        _ => "",
    }
}

/// Keywords that keep their role after a member `.`, such as Rust's `.await`.
pub(super) fn member_keyword(language: Language, token: &str) -> bool {
    match language {
        Language::Rust => token == "await",
        Language::Cpp => token == "template",
        Language::Java => matches!(token, "class" | "new" | "super" | "this"),
        Language::Swift => matches!(token, "init" | "self" | "Self"),
        _ => false,
    }
}

const LANGUAGES: usize = Language::Tsv as usize + 1;

/// A language's sorted words, their roles in the same order, and the longest word.
struct Table {
    words: Box<[&'static str]>,
    roles: Box<[Word]>,
    longest: usize,
}

fn table(language: Language) -> &'static Table {
    static TABLES: [OnceLock<Table>; LANGUAGES] = [const { OnceLock::new() }; LANGUAGES];
    TABLES[language as usize].get_or_init(|| {
        let declares = |list: &str, word: &str| list.split_ascii_whitespace().any(|w| w == word);
        let mut words: Vec<_> = keywords(language)
            .split_ascii_whitespace()
            .map(|word| {
                let expect = if declares(type_words(language), word) {
                    Expect::Type
                } else if declares(function_words(language), word) {
                    Expect::Function
                } else {
                    Expect::Nothing
                };
                let role = STYLE_KEYWORD;
                (word, Word { role, expect })
            })
            .chain(types(language).split_ascii_whitespace().map(|word| {
                let expect = Expect::Nothing;
                (
                    word,
                    Word {
                        role: STYLE_TYPE,
                        expect,
                    },
                )
            }))
            .collect();
        words.sort_unstable_by_key(|(word, _)| *word);
        Table {
            longest: words.iter().map(|(word, _)| word.len()).max().unwrap_or(0),
            roles: words.iter().map(|(_, role)| *role).collect(),
            words: words.into_iter().map(|(word, _)| word).collect(),
        }
    })
}

/// The listed keyword or type `token`, if any. SQL and Batch ignore ASCII case.
pub(super) fn word(language: Language, token: &str) -> Option<Word> {
    let table = table(language);
    // Most names are longer than every listed word.
    if token.len() > table.longest {
        return None;
    }
    let find = |token: &str| {
        table
            .words
            .binary_search(&token)
            .ok()
            .map(|at| table.roles[at])
    };
    if !matches!(language, Language::Sql | Language::Batch) {
        return find(token);
    }
    // Tokens are ASCII; anything longer than the buffer is longer than every word.
    let mut lower = [0; 16];
    let lower = lower.get_mut(..token.len())?;
    lower.copy_from_slice(token.as_bytes());
    lower.make_ascii_lowercase();
    std::str::from_utf8(lower).ok().and_then(find)
}

/// Types, namespaces and type aliases of the C++ standard library named after
/// `std::` (nested namespaces such as `std::chrono::` included).
const STD_TYPES: &str = "allocator allocator_traits any array atomic atomic_flag atomic_ref bad_alloc bad_any_cast bad_cast bad_expected_access bad_function_call bad_optional_access bad_typeid bad_variant_access barrier basic_ifstream basic_istream basic_ofstream basic_ostream basic_string basic_string_view bernoulli_distribution binary_semaphore bitset bool_constant byte chars_format cmatch common_type complex condition_variable condition_variable_any conditional conditional_t copyable_function coroutine_handle counting_semaphore day decay decay_t default_random_engine default_sentinel_t deque directory_entry directory_iterator discrete_distribution divides domain_error duration enable_if enable_if_t equal_to errc error_code error_condition exception expected false_type file_status file_time_type filebuf filesystem_error flat_map flat_multimap flat_multiset flat_set format_args format_context format_error format_string formatter forward_list fstream function future generator greater greater_equal hash high_resolution_clock hours identity ifstream initializer_list int16_t int32_t int64_t int8_t integral_constant intmax_t intptr_t invalid_argument invoke_result invoke_result_t ios ios_base iostream istream istringstream jthread latch length_error less less_equal list lock_guard logic_error map max_align_t mdspan memory_resource microseconds milliseconds minstd_rand minus minutes modulus month move_only_function mt19937 mt19937_64 multimap multiplies multiset mutex nanoseconds negate noop_coroutine_handle normal_distribution not_equal_to nullptr_t numeric_limits ofstream optional ostream ostringstream out_of_range overflow_error packaged_task pair path perms plus polymorphic_allocator priority_queue promise ptrdiff_t queue random_device range_error ratio recursive_directory_iterator recursive_mutex recursive_timed_mutex reference_wrapper regex remove_cv remove_cv_t remove_cvref remove_cvref_t remove_reference remove_reference_t runtime_error scoped_lock seconds seed_seq set shared_future shared_lock shared_mutex shared_ptr shared_timed_mutex size_t smatch source_location space_info span sregex_iterator stack stacktrace steady_clock stop_callback stop_source stop_token streambuf string string_view stringbuf stringstream suspend_always suspend_never system_clock system_error thread time_point timed_mutex to_chars_result from_chars_result true_type tuple type_index type_info u16string u16string_view u32string u32string_view u8string u8string_view uint16_t uint32_t uint64_t uint8_t uintmax_t uintptr_t underflow_error underlying_type uniform_int_distribution uniform_real_distribution unique_lock unique_ptr unordered_map unordered_multimap unordered_multiset unordered_set unreachable_sentinel_t valarray variant vector weak_ptr wregex wstring wstring_view year year_month_day zoned_time";

pub(super) fn std_type(token: &str) -> bool {
    static WORDS: OnceLock<Box<[&'static str]>> = OnceLock::new();
    let words = WORDS.get_or_init(|| {
        let mut words: Vec<_> = STD_TYPES.split_ascii_whitespace().collect();
        words.sort_unstable();
        words.into_boxed_slice()
    });
    words.binary_search(&token).is_ok()
}

/// Preprocessor directive names after `#`. Swift's `#name` forms are all compiler
/// keywords, macros included.
pub(super) fn directive(language: Language, name: &str) -> bool {
    let names = match language {
        Language::C | Language::Cpp => {
            "assert define elif elifdef elifndef else embed endif error ident if ifdef ifndef import include include_next line pragma sccs unassert undef using warning"
        }
        Language::CSharp => {
            "define elif else endif endregion error if line nullable pragma region undef warning"
        }
        Language::Swift => {
            return name
                .as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
        }
        _ => "",
    };
    names.split_ascii_whitespace().any(|word| word == name)
}

/// Directives whose operand is a header name, where `<path>` is a string.
pub(super) fn header_directive(name: &str) -> bool {
    matches!(name, "embed" | "import" | "include" | "include_next")
}

#[cfg(test)]
pub(super) fn listed(language: Language) -> impl Iterator<Item = &'static str> {
    keywords(language)
        .split_ascii_whitespace()
        .chain(types(language).split_ascii_whitespace())
}

#[cfg(test)]
pub(super) fn declaring(language: Language) -> impl Iterator<Item = &'static str> {
    type_words(language)
        .split_ascii_whitespace()
        .chain(function_words(language).split_ascii_whitespace())
}

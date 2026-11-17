//! Structural adapter for the exact Slint 1.18.1 tooltip lowering contract.
use i_slint_compiler::{
    CompilerConfiguration,
    diagnostics::BuildDiagnostics,
    expression_tree::{Callable, Expression, Unit},
    generator::OutputFormat,
    langtype::{Function, Type},
    namedreference::NamedReference,
    object_tree::{Document, ElementRc, PropertyDeclaration, PropertyVisibility},
    parser,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

/// Compile `input` to formatted Rust at `output` and return every file it depends on,
/// exactly as `slint_build::compile_with_config` reports them for Cargo.
pub fn compile(input: &Path, output: &Path) -> Result<Vec<PathBuf>, String> {
    let mut diagnostics = BuildDiagnostics::default();
    let syntax = parser::parse_file(input, &mut diagnostics)
        .ok_or_else(|| diagnostics.to_string_vec().join("\n"))?;
    // This is the constructor/style/domain slint_build uses, including environment/target
    // settings and embedded resources for Rust. The unused slint-build build dependency
    // keeps i-slint-compiler's features identical to a slint_build compile.
    let mut config = CompilerConfiguration::new(OutputFormat::Rust);
    config.style = Some("fluent".into());
    config.translation_domain = std::env::var("CARGO_PKG_NAME").ok();
    let (document, diagnostics, loader) = spin_on::spin_on(i_slint_compiler::compile_syntax_node(
        syntax,
        diagnostics,
        config,
    ));
    if diagnostics.has_errors()
        || (!diagnostics.is_empty() && std::env::var_os("SLINT_COMPILER_DENY_WARNINGS").is_some())
    {
        return Err(diagnostics.to_string_vec().join("\n"));
    }
    let mut dependencies: Vec<PathBuf> = diagnostics
        .all_loaded_files
        .iter()
        .filter(|path| path.is_absolute())
        .cloned()
        .collect();
    for warning in diagnostics.diagnostics_as_string().lines() {
        if !warning.is_empty() {
            println!(
                "cargo:warning={}",
                warning.strip_prefix("warning: ").unwrap_or(warning)
            );
        }
    }
    dependencies.push(input.to_path_buf());
    dependencies.extend(
        document
            .embedded_file_resources
            .borrow()
            .iter()
            .filter_map(|resource| resource.path.as_deref())
            .filter(|path| !path.starts_with("builtin:"))
            .map(PathBuf::from),
    );
    adapt(&document, "TooltipPlacement", "TipBody")?;
    let generated = i_slint_compiler::generator::rust::generate(&document, &loader.compiler_config)
        .map_err(|error| error.to_string())?;
    // Format the generated Rust AST normally. A multi-megabyte single source
    // line makes debug spans and diagnostics expensive for the downstream build.
    let syntax = syn::parse2::<syn::File>(generated).map_err(|error| error.to_string())?;
    std::fs::write(output, prettyplease::unparse(&syntax)).map_err(|error| error.to_string())?;
    Ok(dependencies)
}

fn binary(lhs: Expression, rhs: Expression, op: char) -> Expression {
    Expression::BinaryExpression {
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        op,
        source_location: None,
    }
}
fn body(element: &ElementRc, name: &str) -> Option<ElementRc> {
    let element_ref = element.borrow();
    if element_ref
        .debug
        .iter()
        .any(|debug| debug.type_name == name)
    {
        return Some(element.clone());
    }
    element_ref
        .children
        .iter()
        .find_map(|child| body(child, name))
}

// Reuse the saved stock factory across the initial show and conditional retry.
fn stock_show_call(
    root: &ElementRc,
    name: &str,
    original: Expression,
) -> Result<Expression, String> {
    if original.ty() != Type::Void {
        return Err("The saved stock tooltip show body must return void".into());
    }
    {
        let mut root = root.borrow_mut();
        if root.property_declarations.contains_key(name) {
            return Err(
                "Synthetic tooltip function name collides with an existing declaration".into(),
            );
        }
        root.property_declarations.insert(
            name.into(),
            PropertyDeclaration {
                property_type: Type::Function(Arc::new(Function {
                    return_type: Type::Void,
                    args: Vec::new(),
                    arg_names: Vec::new(),
                })),
                visibility: PropertyVisibility::Private,
                pure: Some(false),
                ..Default::default()
            },
        );
        root.set_binding(name.into(), original.into());
    }
    Ok(Expression::FunctionCall {
        function: Callable::Function(NamedReference::new(root, name.into())),
        arguments: vec![],
        source_location: None,
    })
}

pub fn adapt(document: &Document, global_name: &str, body_name: &str) -> Result<usize, String> {
    let placement = document
        .used_types
        .borrow()
        .globals
        .iter()
        .find(|global| global.id == global_name)
        .cloned()
        .ok_or_else(|| format!("Missing shared {global_name} global"))?;
    let mut components = Vec::new();
    let mut seen = HashSet::new();
    document.visit_all_used_components(|component| {
        i_slint_compiler::object_tree::recurse_elem_including_sub_components(
            component,
            &(),
            &mut |element, _| {
                let enclosing = element.borrow().enclosing_component.upgrade().unwrap();
                if seen.insert(Rc::as_ptr(&enclosing)) {
                    components.push(enclosing);
                }
            },
        );
    });
    let mut count = 0;
    for component in components {
        for popup in component
            .popup_windows
            .borrow()
            .iter()
            .filter(|popup| popup.is_tooltip)
        {
            let content = body(&popup.component.root_element, body_name)
                .ok_or_else(|| format!("Tooltip has no {body_name} content"))?;
            let geometry = content
                .borrow()
                .geometry_props
                .clone()
                .ok_or("Tooltip content has no geometry")?;
            // Native popup updates use layout constraints, and overwrite WindowItem.width/height.
            // Preserve the real content geometry, including aliases moved by compiler passes.
            {
                let mut constraints = popup.component.root_constraints.borrow_mut();
                constraints.min_width = Some(geometry.width.clone());
                constraints.max_width = Some(geometry.width.clone());
                constraints.preferred_width = Some(geometry.width.clone());
                constraints.min_height = Some(geometry.height.clone());
                constraints.max_height = Some(geometry.height.clone());
                constraints.preferred_height = Some(geometry.height.clone());
                constraints.local.min_width = true;
                constraints.local.max_width = true;
                constraints.local.preferred_width = true;
                constraints.local.min_height = true;
                constraints.local.max_height = true;
                constraints.local.preferred_height = true;
            }
            for (coordinate, pointer, dimension, cursor, surface, gap) in [
                (
                    &popup.x,
                    "mouse-x",
                    &geometry.width,
                    "cursor-x",
                    "surface-width",
                    14.0,
                ),
                (
                    &popup.y,
                    "mouse-y",
                    &geometry.height,
                    "cursor-y",
                    "surface-height",
                    18.0,
                ),
            ] {
                let cursor = Expression::PropertyReference(NamedReference::new(
                    &placement.root_element,
                    cursor.into(),
                ));
                let desired = Expression::FunctionCall {
                    function: Callable::Function(NamedReference::new(
                        &placement.root_element,
                        "axis".into(),
                    )),
                    arguments: vec![
                        cursor.clone(),
                        Expression::PropertyReference(dimension.clone()),
                        Expression::PropertyReference(NamedReference::new(
                            &placement.root_element,
                            surface.into(),
                        )),
                        Expression::NumberLiteral(gap, Unit::Px),
                    ],
                    source_location: None,
                };
                let element = coordinate.element();
                let element = element.borrow();
                let mut binding = element
                    .bindings
                    .binding_cell_including_synthetic(coordinate.name())
                    .ok_or("Tooltip coordinate binding missing")?
                    .borrow_mut();
                // desired(host) - cursor(host) + cursor(parent) cancels the actual parent origin,
                // including the runtime-clamped location of an embedded dropdown/options popup.
                binding.expression = binary(
                    binary(desired, cursor, '-'),
                    Expression::PropertyReference(NamedReference::new(
                        &popup.parent_element,
                        pointer.into(),
                    )),
                    '+',
                );
                binding.analysis = None;
            }
            // The native runtime defers geometry tracking to a zero-duration timer.
            // If the anchor moved during the hover delay, synchronize it and retry
            // only the saved stock ShowPopup expression once before the first paint.
            let original = {
                let parent = popup.parent_element.borrow();
                let mut show = parent
                    .bindings
                    .binding_cell_including_synthetic("show")
                    .ok_or("Tooltip show binding missing")?
                    .borrow_mut();
                std::mem::replace(&mut show.expression, Expression::Invalid)
            };
            let original = stock_show_call(
                &component.root_element,
                &format!("__midnight-tooltip-stock-show-{count}"),
                original,
            )?;
            let parent = popup.parent_element.borrow();
            let mut show = parent
                .bindings
                .binding_cell_including_synthetic("show")
                .ok_or("Tooltip show binding missing")?
                .borrow_mut();
            show.expression = Expression::CodeBlock(vec![
                original.clone(),
                Expression::Condition {
                    condition: Box::new(Expression::FunctionCall {
                        function: Callable::Callback(NamedReference::new(
                            &placement.root_element,
                            "synchronize".into(),
                        )),
                        arguments: vec![],
                        source_location: None,
                    }),
                    true_expr: Box::new(original),
                    false_expr: Box::new(Expression::CodeBlock(Vec::new())),
                    source_location: None,
                },
            ]);
            show.analysis = None;
            count += 1;
        }
    }
    if count == 0 {
        return Err("No tooltip sites found; check the pinned compiler contract".into());
    }
    Ok(count)
}

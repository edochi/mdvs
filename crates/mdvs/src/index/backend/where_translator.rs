//! AST-based `--where` clause translator.
//!
//! Parses the user's SQL fragment via [`sqlparser`], walks the `Expr` tree,
//! qualifies bare frontmatter field names with `data.`, rewrites array-field
//! comparisons (`=` / `!=` / `IN` / `NOT IN`) to `array_has(...)` forms that
//! Lance can execute, rejects `Array(Float)` references up front, and emits
//! canonical SQL via [`Expr::to_string`].
//!
//! The translator returns both the rewritten clause and a list of
//! [`WhereRewrite`] entries (one per array-field rewrite that fired) so the
//! caller can surface a translation note to the user.
//!
//! ## Design
//!
//! The previous shape was a regex pass: two regexes (`lit` for literals,
//! `ident` for identifiers) and a walk-and-rewrite loop. Operator-blind by
//! construction — to detect `tags = 'rust'` and rewrite it to
//! `array_has(tags, 'rust')` would have required peek-ahead regexes,
//! function-call heuristics, and special IN-list parsing. Each rule
//! compounded fragility.
//!
//! The AST-based shape gets structural awareness for free:
//! `Expr::BinaryOp { op: Eq, left: Identifier, right: Value(...) }` is
//! exactly the pattern we want to rewrite; an `Identifier` nested inside an
//! `Expr::Function` is structurally distinct from a top-level operand, so
//! `array_length(tags) > 2` never triggers the array-equality rewrite. See
//! TODO-0191 for the full rationale.
//!
//! ## Canonicalization
//!
//! `Expr::to_string()` emits canonical SQL — `AND` / `OR` / `BETWEEN` are
//! uppercased, typed literals (`DATE '…'`, `TIMESTAMP '…'`) keep their
//! keyword, redundant whitespace is normalized. Semantics are preserved;
//! whitespace and case may differ from the user's input. The translation
//! note shows both forms so the user can see what changed.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlparser::ast::{
    BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments,
    Ident, ObjectName, ObjectNamePart, UnaryOperator,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use super::search::{RESERVED_COLS, SQL_KEYWORDS};

/// A single array-field comparison that was auto-rewritten. Surfaced to the
/// user in the search outcome's "Note" block so the rewrite isn't magic.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WhereRewrite {
    /// The original sub-expression as written by the user (canonicalized via
    /// `Expr::to_string` — case and whitespace may differ from the literal
    /// input).
    pub original: String,
    /// The `array_has(...)` form mdvs sent to Lance.
    pub rewritten: String,
}

/// Result of translating a `--where` clause.
#[derive(Debug, Clone)]
pub struct TranslatedWhere {
    /// The rewritten clause to hand to Lance.
    pub clause: String,
    /// Array-field rewrites that fired. Empty if the clause needed no
    /// rewriting.
    pub rewrites: Vec<WhereRewrite>,
}

/// Schema-aware `--where` translator. Frontmatter fields (children of the
/// `data` Struct) are prefixed with `data.`; genuine internal columns are
/// left top-level. A name that is *both* a frontmatter field and an internal
/// column is a collision: it's resolved toward the frontmatter field when
/// `internal_prefix`/aliases give the internal column another name,
/// otherwise it errors (mirroring the original regex translator). Comparisons
/// against `Array(*)` fields (other than `Array(Float)`, which is rejected
/// up front) are rewritten to `array_has(...)` forms.
pub(super) fn translate_where_to_struct(
    clause: &str,
    data_children: &HashSet<String>,
    float_list_fields: &HashSet<String>,
    array_fields: &HashSet<String>,
    internal_prefix: &str,
    aliases: &HashMap<String, String>,
) -> anyhow::Result<TranslatedWhere> {
    // Empty / whitespace-only input → caller convention is empty in, empty
    // out (the search method treats `None` as "no filter").
    if clause.trim().is_empty() {
        return Ok(TranslatedWhere {
            clause: String::new(),
            rewrites: vec![],
        });
    }

    // Reverse alias lookup: alias name -> real internal column.
    let alias_to_internal: HashMap<&str, &str> = aliases
        .iter()
        .map(|(col, alias)| (alias.as_str(), col.as_str()))
        .collect();

    let ctx = WalkCtx {
        data_children,
        float_list_fields,
        array_fields,
        alias_to_internal,
        internal_prefix,
    };

    let mut expr = Parser::new(&GenericDialect {})
        .try_with_sql(clause)
        .map_err(|e| anyhow::anyhow!("failed to tokenize --where '{clause}': {e}"))?
        .parse_expr()
        .map_err(|e| anyhow::anyhow!("failed to parse --where '{clause}': {e}"))?;

    let mut rewrites = Vec::new();
    walk_expr(&mut expr, &ctx, &mut rewrites)?;

    Ok(TranslatedWhere {
        clause: expr.to_string(),
        rewrites,
    })
}

struct WalkCtx<'a> {
    data_children: &'a HashSet<String>,
    float_list_fields: &'a HashSet<String>,
    array_fields: &'a HashSet<String>,
    alias_to_internal: HashMap<&'a str, &'a str>,
    internal_prefix: &'a str,
}

impl<'a> WalkCtx<'a> {
    fn has_aliasing(&self) -> bool {
        !self.internal_prefix.is_empty() || !self.alias_to_internal.is_empty()
    }
}

/// Walk the AST in-place. At each node:
/// 1. Try to apply an array-field rewrite (replaces the node entirely).
/// 2. Recurse into children, qualifying any bare identifiers with `data.`
///    and rejecting `Array(Float)` references.
fn walk_expr(
    expr: &mut Expr,
    ctx: &WalkCtx,
    rewrites: &mut Vec<WhereRewrite>,
) -> anyhow::Result<()> {
    // Pre-rewrite check: is this whole node an array-field comparison we
    // should convert to array_has(...)? If so, replace it and capture the
    // before/after pair for the translation note.
    if let Some(new_expr) = try_array_rewrite(expr, ctx)? {
        let original = expr.to_string();
        let rewritten = new_expr.to_string();
        rewrites.push(WhereRewrite {
            original,
            rewritten,
        });
        *expr = new_expr;
        // Fall through and recurse into children of the new expr to qualify
        // the array_has(...) argument identifiers. The rewritten form puts
        // `tags` inside a Function; the recursion won't re-trigger
        // try_array_rewrite (it only fires on BinaryOp/InList, not Function
        // arguments).
    }

    match expr {
        Expr::Identifier(ident) => qualify_single_ident(ident, ctx),
        Expr::CompoundIdentifier(parts) => qualify_compound(parts, ctx),
        Expr::BinaryOp { left, right, .. } => {
            walk_expr(left, ctx, rewrites)?;
            walk_expr(right, ctx, rewrites)
        }
        Expr::UnaryOp { expr: inner, .. } => walk_expr(inner, ctx, rewrites),
        Expr::Nested(inner) => walk_expr(inner, ctx, rewrites),
        Expr::IsNull(inner) | Expr::IsNotNull(inner) => walk_expr(inner, ctx, rewrites),
        Expr::IsTrue(inner)
        | Expr::IsNotTrue(inner)
        | Expr::IsFalse(inner)
        | Expr::IsNotFalse(inner)
        | Expr::IsUnknown(inner)
        | Expr::IsNotUnknown(inner) => walk_expr(inner, ctx, rewrites),
        Expr::Between {
            expr: e, low, high, ..
        } => {
            walk_expr(e, ctx, rewrites)?;
            walk_expr(low, ctx, rewrites)?;
            walk_expr(high, ctx, rewrites)
        }
        Expr::InList { expr: e, list, .. } => {
            walk_expr(e, ctx, rewrites)?;
            for item in list.iter_mut() {
                walk_expr(item, ctx, rewrites)?;
            }
            Ok(())
        }
        Expr::Like {
            expr: e,
            pattern,
            escape_char: _,
            ..
        } => {
            walk_expr(e, ctx, rewrites)?;
            walk_expr(pattern, ctx, rewrites)
        }
        Expr::ILike {
            expr: e, pattern, ..
        } => {
            walk_expr(e, ctx, rewrites)?;
            walk_expr(pattern, ctx, rewrites)
        }
        Expr::Function(func) => {
            // Recurse into function arguments to qualify any column references.
            if let FunctionArguments::List(arg_list) = &mut func.args {
                for arg in arg_list.args.iter_mut() {
                    if let FunctionArg::Unnamed(FunctionArgExpr::Expr(inner)) = arg {
                        walk_expr(inner, ctx, rewrites)?;
                    } else if let FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(inner),
                        ..
                    } = arg
                    {
                        walk_expr(inner, ctx, rewrites)?;
                    }
                }
            }
            Ok(())
        }
        Expr::Cast { expr: inner, .. } => walk_expr(inner, ctx, rewrites),
        Expr::Value(_) | Expr::TypedString { .. } => Ok(()),
        // Other variants (subqueries, CASE, INTERVAL, JSON ops, array
        // constructors, etc.) we don't touch — anything user-written that
        // contains a bare identifier of ours and falls in one of these
        // shapes will be passed through to Lance unchanged. The set of
        // walked variants covers everything mdvs's existing test suite
        // exercises plus the four new array-equality rewrite cases.
        _ => Ok(()),
    }
}

fn qualify_single_ident(ident: &mut Ident, ctx: &WalkCtx) -> anyhow::Result<()> {
    let name = ident.value.as_str();

    // SQL keywords / function-like tokens — leave alone.
    if SQL_KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(name)) {
        return Ok(());
    }

    // Internal column accessed via alias.
    if let Some(internal) = ctx.alias_to_internal.get(name) {
        ident.value = (*internal).to_string();
        return Ok(());
    }

    // Internal column accessed via prefix.
    if !ctx.internal_prefix.is_empty()
        && let Some(stripped) = name.strip_prefix(ctx.internal_prefix)
        && RESERVED_COLS.contains(&stripped)
    {
        ident.value = stripped.to_string();
        return Ok(());
    }

    // Array(Float) rejection — the column is unfilterable.
    if ctx.float_list_fields.contains(name) {
        return Err(array_float_error(name));
    }

    let is_reserved = RESERVED_COLS.contains(&name);
    let is_frontmatter = ctx.data_children.contains(name);

    if is_reserved && is_frontmatter {
        if !ctx.has_aliasing() {
            return Err(ambiguous_column_error(name));
        }
        // With aliasing, bare name resolves to frontmatter — qualify.
        // We can't mutate `Ident` into a CompoundIdentifier in place; the
        // caller's enclosing Expr is what holds the variant. To work around
        // this without restructuring the walk, we encode `data.<name>` as a
        // dotted value inside a single Ident — sqlparser's display emits the
        // value verbatim, which for our purposes is fine because the
        // resulting clause is reparseable. The same trick is used below.
        ident.value = format!("data.{name}");
    } else if !is_reserved {
        ident.value = format!("data.{name}");
    }
    // is_reserved && !is_frontmatter → genuine internal column, leave alone.

    Ok(())
}

fn qualify_compound(parts: &mut Vec<Ident>, ctx: &WalkCtx) -> anyhow::Result<()> {
    let Some(first) = parts.first() else {
        return Ok(());
    };
    let first_name = first.value.as_str();

    // Already `data.` qualified — verify the segment after `data.` isn't an
    // Array(Float) field (those would panic Lance), then leave alone.
    if first_name == "data" {
        if let Some(second) = parts.get(1)
            && ctx.float_list_fields.contains(second.value.as_str())
        {
            return Err(array_float_error(second.value.as_str()));
        }
        return Ok(());
    }

    // Internal column via alias (e.g. `fid.something` if alias maps to a
    // dotted path — uncommon but mirrors the regex translator's behavior).
    if let Some(internal) = ctx.alias_to_internal.get(first_name) {
        parts[0].value = (*internal).to_string();
        return Ok(());
    }

    // Prefix-resolved internal column.
    if !ctx.internal_prefix.is_empty()
        && let Some(stripped) = first_name.strip_prefix(ctx.internal_prefix)
        && RESERVED_COLS.contains(&stripped)
    {
        parts[0].value = stripped.to_string();
        return Ok(());
    }

    // Array(Float) rejection — the leading segment is the field name.
    if ctx.float_list_fields.contains(first_name) {
        return Err(array_float_error(first_name));
    }

    let is_reserved = RESERVED_COLS.contains(&first_name);
    let is_frontmatter = ctx.data_children.contains(first_name);

    if is_reserved && is_frontmatter {
        if !ctx.has_aliasing() {
            return Err(ambiguous_column_error(first_name));
        }
        parts.insert(0, Ident::new("data"));
    } else if !is_reserved {
        parts.insert(0, Ident::new("data"));
    }
    Ok(())
}

/// If `expr` is an array-field equality (`=`, `!=`, `IN`, `NOT IN`) against
/// scalar literals, return the rewritten `array_has(...)` form. Otherwise
/// return `Ok(None)`.
fn try_array_rewrite(expr: &Expr, ctx: &WalkCtx) -> anyhow::Result<Option<Expr>> {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            let is_eq = matches!(op, BinaryOperator::Eq);
            let is_neq = matches!(op, BinaryOperator::NotEq);
            if !is_eq && !is_neq {
                return Ok(None);
            }
            // Identify which side is the array-field identifier and which is
            // the scalar literal. Both orderings supported.
            let (field_name, literal) = if let (Some(f), Some(l)) =
                (array_field_name(left, ctx), as_literal(right))
            {
                (f, l)
            } else if let (Some(f), Some(l)) = (array_field_name(right, ctx), as_literal(left)) {
                (f, l)
            } else {
                return Ok(None);
            };
            let array_has = make_array_has(&field_name, literal);
            Ok(Some(if is_eq { array_has } else { negate(array_has) }))
        }
        Expr::InList {
            expr: e,
            list,
            negated,
        } => {
            let Some(field_name) = array_field_name(e, ctx) else {
                return Ok(None);
            };
            // Every list entry must be a scalar literal — if any isn't, fall
            // back to the un-rewritten clause (Lance will error if needed).
            let mut literals = Vec::with_capacity(list.len());
            for item in list {
                let Some(lit) = as_literal(item) else {
                    return Ok(None);
                };
                literals.push(lit);
            }
            // `literals` is guaranteed non-empty here: the early returns
            // above bail out on empty / non-literal IN lists. `pop` returns
            // None only when the source is empty.
            let mut iter = literals.into_iter();
            let Some(first_lit) = iter.next() else {
                return Ok(None);
            };
            // Build (array_has(f, v1) OR array_has(f, v2) OR ...).
            let first = make_array_has(&field_name, first_lit);
            let or_chain = iter.fold(first, |acc, lit| Expr::BinaryOp {
                left: Box::new(acc),
                op: BinaryOperator::Or,
                right: Box::new(make_array_has(&field_name, lit)),
            });
            Ok(Some(if *negated {
                Expr::Nested(Box::new(negate(Expr::Nested(Box::new(or_chain)))))
            } else {
                or_chain
            }))
        }
        _ => Ok(None),
    }
}

/// If `expr` is a bare identifier (`tags`) or a `data.<x>` compound that
/// names a known array field, return the unqualified field name (so the
/// rewrite can re-emit it as a `data.`-qualified arg to `array_has`).
/// Returns `None` for anything else — function calls, literals, nested
/// expressions, identifiers that don't name array fields.
fn array_field_name(expr: &Expr, ctx: &WalkCtx) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => {
            let name = &ident.value;
            if ctx.array_fields.contains(name) {
                Some(name.clone())
            } else {
                None
            }
        }
        Expr::CompoundIdentifier(parts) => {
            // Accept `data.<x>` where <x> is an array field.
            if parts.len() == 2
                && parts[0].value == "data"
                && ctx.array_fields.contains(&parts[1].value)
            {
                Some(parts[1].value.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Is this expression a scalar literal we can use as the second arg of
/// `array_has`? Accepts string / number / boolean / typed-string (`DATE`,
/// `TIMESTAMP`) literals.
fn as_literal(expr: &Expr) -> Option<Expr> {
    match expr {
        Expr::Value(_) | Expr::TypedString { .. } => Some(expr.clone()),
        // A unary minus on a number — `-5` parses as UnaryOp(Minus, Value(5))
        // — is also a scalar literal for our purposes.
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr: inner,
        } if matches!(inner.as_ref(), Expr::Value(_)) => Some(expr.clone()),
        _ => None,
    }
}

/// Construct `array_has(data.<field_name>, <literal>)`.
fn make_array_has(field_name: &str, literal: Expr) -> Expr {
    Expr::Function(sqlparser::ast::Function {
        name: ObjectName(vec![ObjectNamePart::Identifier(Ident::new("array_has"))]),
        uses_odbc_syntax: false,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args: vec![
                FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::CompoundIdentifier(vec![
                    Ident::new("data"),
                    Ident::new(field_name),
                ]))),
                FunctionArg::Unnamed(FunctionArgExpr::Expr(literal)),
            ],
            clauses: vec![],
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
    })
}

fn negate(expr: Expr) -> Expr {
    Expr::UnaryOp {
        op: UnaryOperator::Not,
        expr: Box::new(expr),
    }
}

fn array_float_error(name: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "filtering on Array(Float) field '{name}' is not supported in --where. \
         Filter on a different field or store the values in a parallel scalar field."
    )
}

fn ambiguous_column_error(name: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "ambiguous column '{name}' in --where: it is both a frontmatter field and \
         an internal column. Disambiguate by setting [search].internal_prefix \
         (e.g. \"_\") or [search.aliases].{name} = \"<alias>\""
    )
}

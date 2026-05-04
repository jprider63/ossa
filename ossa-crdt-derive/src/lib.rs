use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, GenericParam, Ident, Type, WherePredicate};

#[proc_macro_derive(CRDT, attributes(crdt))]
pub fn crdt_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();
    impl_crdt_derive(&ast).into()
}

struct CrdtAttrs {
    time: Option<Type>,
    bounds: Vec<WherePredicate>,
    bounds_concretize_time: Vec<WherePredicate>,
    concretize_time: bool,
    concretize_time_op: bool,
}

fn parse_crdt_attrs(ast: &DeriveInput) -> CrdtAttrs {
    let mut attrs = CrdtAttrs {
        time: None,
        bounds: Vec::new(),
        bounds_concretize_time: Vec::new(),
        concretize_time: false,
        concretize_time_op: false,
    };

    for attr in &ast.attrs {
        if attr.path().is_ident("crdt") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("time") {
                    let value = meta.value()?;
                    let ty: Type = value.parse()?;
                    attrs.time = Some(ty);
                    Ok(())
                } else if meta.path.is_ident("bound") {
                    let value = meta.value()?;
                    let bound_str: syn::LitStr = value.parse()?;
                    let predicate: WherePredicate = syn::parse_str(&bound_str.value())
                        .expect("failed to parse #[crdt(bound = \"...\")] as a where predicate");
                    attrs.bounds.push(predicate);
                    Ok(())
                } else if meta.path.is_ident("bound_concretize_time") {
                    let value = meta.value()?;
                    let bound_str: syn::LitStr = value.parse()?;
                    let predicate: WherePredicate = syn::parse_str(&bound_str.value())
                        .expect("failed to parse #[crdt(bound_concretize_time = \"...\")] as a where predicate");
                    attrs.bounds_concretize_time.push(predicate);
                    Ok(())
                } else if meta.path.is_ident("concretize_time") {
                    attrs.concretize_time = true;
                    Ok(())
                } else if meta.path.is_ident("concretize_time_op") {
                    attrs.concretize_time_op = true;
                    Ok(())
                } else {
                    Err(meta.error("unrecognized crdt attribute"))
                }
            })
            .expect("failed to parse #[crdt(...)] attribute");
        }
    }

    attrs
}

/// Find the generic type parameter that represents "Time" on the struct.
/// Returns the Ident if found.
fn find_time_type_param(ast: &DeriveInput) -> Option<&Ident> {
    ast.generics.params.iter().find_map(|p| match p {
        GenericParam::Type(t) if t.ident == "Time" => Some(&t.ident),
        _ => None,
    })
}

fn impl_crdt_derive(ast: &DeriveInput) -> TokenStream {
    let fields = match &ast.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => &named.named,
            _ => panic!("CRDT derive only supports structs with named fields"),
        },
        _ => panic!("CRDT derive only supports structs"),
    };

    assert!(!fields.is_empty(), "CRDT derive requires at least one field");

    let struct_name = &ast.ident;
    let vis = &ast.vis;
    let op_enum_name = format_ident!("{}Op", struct_name);

    // Collect field info: (field_name, field_type, VariantName).
    let field_info: Vec<_> = fields
        .iter()
        .map(|f| {
            let field_name = f.ident.as_ref().unwrap();
            let field_type = &f.ty;
            let variant_name =
                format_ident!("{}", field_name.to_string().to_upper_camel_case());
            (field_name, field_type, variant_name)
        })
        .collect();

    // Parse #[crdt(...)] attributes.
    let crdt_attrs = parse_crdt_attrs(ast);
    let extra_bounds = &crdt_attrs.bounds;
    let extra_bounds_concretize_time = &crdt_attrs.bounds_concretize_time;

    // Determine Time type.
    let time_type: Type = crdt_attrs.time.unwrap_or_else(|| {
        let first_field_type = field_info[0].1;
        syn::parse_quote!(<#first_field_type as ossa_crdt::CRDT>::Time)
    });

    // Generate the Op enum variants.
    let enum_variants = field_info.iter().map(|(_, field_type, variant_name)| {
        quote! {
            #variant_name(<#field_type as ossa_crdt::CRDT>::Op)
        }
    });

    // Handle generics.
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // Build a combined where clause that merges existing predicates with extra bounds.
    // This ensures `where` is always emitted when there are any predicates.
    let combined_where_clause = {
        let existing: Vec<_> = where_clause
            .iter()
            .flat_map(|w| w.predicates.iter())
            .collect();
        if existing.is_empty() && extra_bounds.is_empty() {
            quote! {}
        } else {
            quote! { where #(#existing,)* #(#extra_bounds,)* }
        }
    };

    let type_params = &ast.generics.params;
    let abbr_type_args: Vec<_> = ast
        .generics
        .params
        .iter()
        .map(|p| match p {
            GenericParam::Lifetime(l) => {
                let lt = &l.lifetime;
                quote!(#lt)
            }
            GenericParam::Type(t) => {
                let name = &t.ident;
                quote!(#name)
            }
            GenericParam::Const(c) => {
                let name = &c.ident;
                quote!(#name)
            }
        })
        .collect();

    let has_generics = !abbr_type_args.is_empty();
    let op_ty_generics = if has_generics {
        quote!(<#(#abbr_type_args),*>)
    } else {
        quote!()
    };

    // Where clause bounds for the Op enum.
    let existing_where_predicates = where_clause.map(|w| {
        let predicates = &w.predicates;
        quote! { #predicates }
    });

    let crdt_bounds = field_info.iter().map(|(_, field_type, _)| {
        quote! {
            #field_type: ossa_crdt::CRDT
        }
    });

    let op_serde_bounds = field_info.iter().map(|(_, field_type, _)| {
        quote! {
            <#field_type as ossa_crdt::CRDT>::Op: ::serde::Serialize + for<'deserialize> ::serde::Deserialize<'deserialize>
        }
    });
    let op_serde_bounds = quote! {
        #(#op_serde_bounds,)*
    };
    let op_serde_bounds = op_serde_bounds.to_string();

    let op_enum = quote! {
        #[derive(::derive_more::Debug, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(bound = #op_serde_bounds)]
        #[automatically_derived]
        #vis enum #op_enum_name<#type_params>
        where
            #existing_where_predicates
            #(#crdt_bounds,)*
            // #(#op_serde_bounds,)*
        {
            #(#enum_variants),*
        }
    };

    // Generate match arms for the CRDT impl.
    let field_count = field_info.len();
    let match_arms_impl = field_info.iter().map(|(field_name, _, variant_name)| {
        // Don't use record update syntax for single field structs.
        if field_count == 1 {
            quote! {
                #op_enum_name::#variant_name(__op) => #struct_name {
                    #field_name: self.#field_name.apply(causal_state, __op),
                }
            }
        } else {
            quote! {
                #op_enum_name::#variant_name(__op) => #struct_name {
                    #field_name: self.#field_name.apply(causal_state, __op),
                    ..self
                }
            }
        }
    });

    let crdt_impl = quote! {
        #[automatically_derived]
        impl #impl_generics ossa_crdt::CRDT for #struct_name #ty_generics #combined_where_clause {
            type Op = #op_enum_name #op_ty_generics;
            type Time = #time_type;

            fn apply<__CS: ossa_crdt::time::CausalState<Time = Self::Time>>(
                self,
                causal_state: &__CS,
                op: Self::Op,
            ) -> Self {
                match op {
                    #(#match_arms_impl),*
                }
            }
        }
    };

    // Build a combined where clause for ConcretizeTime that also includes bound_concretize_time predicates.
    let concretize_time_where_clause = {
        let existing: Vec<_> = where_clause
            .iter()
            .flat_map(|w| w.predicates.iter())
            .collect();
        let all_bounds: Vec<_> = extra_bounds.iter()
            .chain(extra_bounds_concretize_time.iter())
            .collect();
        if existing.is_empty() && all_bounds.is_empty() {
            quote! {}
        } else {
            quote! { where #(#existing,)* #(#all_bounds,)* }
        }
    };

    // Generate ConcretizeTime impls if enabled via attributes.
    let concretize_time_op_impl = if crdt_attrs.concretize_time_op {
        gen_op_concretize_time_impl(ast, &field_info, &op_enum_name, &concretize_time_where_clause)
    } else {
        quote! {}
    };
    let concretize_time_struct_impl = if crdt_attrs.concretize_time {
        gen_struct_concretize_time_impl(ast, struct_name, &field_info, &concretize_time_where_clause)
    } else {
        quote! {}
    };

    quote! {
        #op_enum
        #crdt_impl
        #concretize_time_op_impl
        #concretize_time_struct_impl
    }
}

/// Generate a `ConcretizeTime` impl for the Op enum.
///
/// Only generated when the struct has a generic type parameter named `Time`.
/// Produces:
/// ```ignore
/// impl<__HeaderId, Time: ConcretizeTime<__HeaderId>, ...>
///     ConcretizeTime<__HeaderId> for FooOp<Time, ...>
/// {
///     type Serialized = FooOp<Time::Serialized, ...>;
///     fn concretize_time(src: Self::Serialized, current_header: __HeaderId) -> Self { ... }
/// }
/// ```
fn gen_op_concretize_time_impl(
    ast: &DeriveInput,
    field_info: &[(&Ident, &Type, Ident)],
    op_enum_name: &Ident,
    combined_where_clause: &TokenStream,
) -> TokenStream {
    let time_param = match find_time_type_param(ast) {
        Some(tp) => tp,
        None => return quote! {},
    };

    let header_id = format_ident!("__HeaderId");

    // Build the generic params for the impl, adding __HeaderId and updating Time's bounds.
    // Other type params are passed through as-is.
    let impl_params: Vec<_> = std::iter::once(quote! { #header_id })
        .chain(ast.generics.params.iter().map(|p| match p {
            GenericParam::Type(t) if t.ident == *time_param => {
                let ident = &t.ident;
                let existing_bounds = t.bounds.iter().collect::<Vec<_>>();
                let bounds_tokens = if existing_bounds.is_empty() {
                    quote! {}
                } else {
                    quote! { + #(#existing_bounds)+* }
                };
                quote! {
                    #ident: ossa_crdt::time::ConcretizeTime<#header_id> #bounds_tokens
                }
            }
            other => quote! { #other },
        }))
        .collect();

    // Build the abbreviated type args for the Op enum, with Time replaced by Time::Serialized.
    let serialized_type_args: Vec<_> = ast
        .generics
        .params
        .iter()
        .map(|p| match p {
            GenericParam::Type(t) if t.ident == *time_param => {
                let ident = &t.ident;
                quote!(<#ident as ossa_crdt::time::ConcretizeTime<#header_id>>::Serialized)
            }
            GenericParam::Lifetime(l) => {
                let lt = &l.lifetime;
                quote!(#lt)
            }
            GenericParam::Type(t) => {
                let name = &t.ident;
                quote!(#name)
            }
            GenericParam::Const(c) => {
                let name = &c.ident;
                quote!(#name)
            }
        })
        .collect();

    // The regular type args (unchanged).
    let regular_type_args: Vec<_> = ast
        .generics
        .params
        .iter()
        .map(|p| match p {
            GenericParam::Lifetime(l) => {
                let lt = &l.lifetime;
                quote!(#lt)
            }
            GenericParam::Type(t) => {
                let name = &t.ident;
                quote!(#name)
            }
            GenericParam::Const(c) => {
                let name = &c.ident;
                quote!(#name)
            }
        })
        .collect();

    // Generate match arms for concretize_time.
    // let field_count = field_info.len();
    let match_arms: Vec<_> = field_info
        .iter()
        .map(|(_, field_type, variant_name)| {
            let header_expr = quote!(current_header);
            quote! {
                #op_enum_name::#variant_name(__inner) => {
                    #op_enum_name::#variant_name(
                        <<#field_type as ossa_crdt::CRDT>::Op as ossa_crdt::time::ConcretizeTime<#header_id>>::concretize_time(__inner, #header_expr)
                    )
                }
            }
        })
        .collect();

    quote! {
        #[automatically_derived]
        impl<#(#impl_params),*> ossa_crdt::time::ConcretizeTime<#header_id> for #op_enum_name<#(#regular_type_args),*>
            #combined_where_clause
        {
            type Serialized = #op_enum_name<#(#serialized_type_args),*>;

            fn concretize_time(src: Self::Serialized, current_header: #header_id) -> Self {
                match src {
                    #(#match_arms),*
                }
            }
        }
    }
}

/// Generate a `ConcretizeTime` impl for the struct itself.
///
/// Only generated when the struct has a generic type parameter named `Time`.
/// Produces:
/// ```ignore
/// impl<__HeaderId: Clone, Time: ConcretizeTime<__HeaderId>, ...>
///     ConcretizeTime<__HeaderId> for Foo<Time, ...>
/// {
///     type Serialized = Foo<Time::Serialized, ...>;
///     fn concretize_time(src: Self::Serialized, current_header: __HeaderId) -> Self { ... }
/// }
/// ```
fn gen_struct_concretize_time_impl(
    ast: &DeriveInput,
    struct_name: &Ident,
    field_info: &[(&Ident, &Type, Ident)],
    combined_where_clause: &TokenStream,
) -> TokenStream {
    let time_param = match find_time_type_param(ast) {
        Some(tp) => tp,
        None => return quote! {},
    };

    let header_id = format_ident!("__HeaderId");

    let needs_clone = field_info.len() > 1;

    // Build the generic params for the impl, adding __HeaderId (with Clone if needed) and updating Time's bounds.
    let header_id_param = if needs_clone {
        quote! { #header_id: Clone }
    } else {
        quote! { #header_id }
    };
    let impl_params: Vec<_> = std::iter::once(header_id_param)
        .chain(ast.generics.params.iter().map(|p| match p {
            GenericParam::Type(t) if t.ident == *time_param => {
                let ident = &t.ident;
                let existing_bounds = t.bounds.iter().collect::<Vec<_>>();
                let bounds_tokens = if existing_bounds.is_empty() {
                    quote! {}
                } else {
                    quote! { + #(#existing_bounds)+* }
                };
                quote! {
                    #ident: ossa_crdt::time::ConcretizeTime<#header_id> #bounds_tokens
                }
            }
            other => quote! { #other },
        }))
        .collect();

    // Build type args with Time replaced by Time::Serialized.
    let serialized_type_args: Vec<_> = ast
        .generics
        .params
        .iter()
        .map(|p| match p {
            GenericParam::Type(t) if t.ident == *time_param => {
                let ident = &t.ident;
                quote!(<#ident as ossa_crdt::time::ConcretizeTime<#header_id>>::Serialized)
            }
            GenericParam::Lifetime(l) => {
                let lt = &l.lifetime;
                quote!(#lt)
            }
            GenericParam::Type(t) => {
                let name = &t.ident;
                quote!(#name)
            }
            GenericParam::Const(c) => {
                let name = &c.ident;
                quote!(#name)
            }
        })
        .collect();

    // The regular type args (unchanged).
    let regular_type_args: Vec<_> = ast
        .generics
        .params
        .iter()
        .map(|p| match p {
            GenericParam::Lifetime(l) => {
                let lt = &l.lifetime;
                quote!(#lt)
            }
            GenericParam::Type(t) => {
                let name = &t.ident;
                quote!(#name)
            }
            GenericParam::Const(c) => {
                let name = &c.ident;
                quote!(#name)
            }
        })
        .collect();

    // Generate field assignments: each field calls ConcretizeTime on itself.
    let field_count = field_info.len();
    let field_assignments: Vec<_> = field_info
        .iter()
        .enumerate()
        .map(|(i, (field_name, field_type, _))| {
            let header_expr = if i < field_count - 1 {
                quote!(current_header.clone())
            } else {
                quote!(current_header)
            };
            quote! {
                #field_name: <#field_type as ossa_crdt::time::ConcretizeTime<#header_id>>::concretize_time(src.#field_name, #header_expr)
            }
        })
        .collect();

    quote! {
        #[automatically_derived]
        impl<#(#impl_params),*> ossa_crdt::time::ConcretizeTime<#header_id> for #struct_name<#(#regular_type_args),*>
            #combined_where_clause
        {
            type Serialized = #struct_name<#(#serialized_type_args),*>;

            fn concretize_time(src: Self::Serialized, current_header: #header_id) -> Self {
                #struct_name {
                    #(#field_assignments),*
                }
            }
        }
    }
}

use proc_macro2::TokenStream;
use quote::quote;
use std::collections::HashSet;
use syn::{
    punctuated::Punctuated, token::Comma, AttrStyle, Attribute, Data, Expr, Field, Fields,
    GenericParam, Ident, Lit, Meta, Type,
};

#[proc_macro_derive(Typeable, attributes(tag))]
pub fn typeable_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse(input).unwrap();

    // Build the trait implementation
    impl_typeable_macro(&ast)
}

fn impl_type_ident(t: &Type) -> TokenStream {
    quote! {
        h.update(<#t as ossa_typeable::Typeable>::type_ident().identifier());
    }
}

// Skip types like `PhantomData` and eventually check attributes.
fn skip_field(f: &Field) -> bool {
    fn extract_type_name(t: &Type) -> Option<&Ident> {
        match t {
            Type::Path(p) => {
                let segment = p.path.segments.first()?;
                Some(&segment.ident)
            }
            _ => None,
        }
    }
    let type_name = extract_type_name(&f.ty);

    type_name.map_or(false, |n| n == "PhantomData")
}

fn impl_fields(fs: &Fields) -> TokenStream {
    let empty = Punctuated::<Field, Comma>::new();
    let (tag, fields) = match fs {
        Fields::Named(fs) => (0u8, fs.named.iter()),
        Fields::Unnamed(fs) => (1, fs.unnamed.iter()),
        Fields::Unit => (0, empty.iter()),
    };

    // Filter any fields.
    let mut fields = fields.filter(|f| !skip_field(&f)).collect::<Vec<_>>();

    let c = fields.len();

    let fields = match &fs {
        Fields::Named(_) => {
            // Sort the fields by their name.
            fields.sort_by_key(|f| f.ident.as_ref());

            TokenStream::from_iter(fields.iter().map(|f| {
                let name = f.ident.as_ref().unwrap().to_string();
                let ty = impl_type_ident(&f.ty);

                quote! {
                    helper_string(&mut h, #name);
                    #ty
                }
            }))
        }
        Fields::Unnamed(_) => TokenStream::from_iter(fields.iter().map(|f| impl_type_ident(&f.ty))),
        Fields::Unit => {
            quote! {}
        }
    };

    quote! {
        helper_u8(&mut h, #tag);
        helper_counter(&mut h, #c);
        #fields
    }
}

fn impl_tag(attrs: &[Attribute]) -> TokenStream {
    fn get_tag(a: &Attribute) -> Option<String> {
        match &a.meta {
            Meta::NameValue(v) => {
                let seg = v.path.segments.first()?;
                if &seg.ident.to_string() == "tag" {
                    match &v.value {
                        Expr::Lit(l) => match &l.lit {
                            Lit::Str(s) => Some(s.value()),
                            _ => None,
                        },
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    if let Some(attr) = attrs.iter().find_map(|a| {
        if a.style == AttrStyle::Outer {
            get_tag(a)
        } else {
            None
        }
    }) {
        quote! {
            helper_string(&mut h, #attr);
        }
    } else {
        quote! {}
    }
}

fn impl_where(ast: &syn::DeriveInput) -> TokenStream {
    fn extract_fields_types(fs: Fields) -> impl Iterator<Item = Type> {
        let empty = Punctuated::<Field, Comma>::new();
        let fields = match fs {
            Fields::Named(n) => n.named,
            Fields::Unnamed(u) => u.unnamed,
            Fields::Unit => empty,
        };
        fields.into_iter().filter(|f| !skip_field(&f)).map(|f| f.ty)
    }

    fn extract_field_types(d: &Data) -> HashSet<Type> {
        match d {
            Data::Struct(s) => extract_fields_types(s.fields.clone()).collect(),
            Data::Enum(e) => e
                .variants
                .iter()
                .flat_map(|v| extract_fields_types(v.fields.clone()))
                .collect(),
            Data::Union(_) => {
                panic!("Union types are not supported.")
            }
        }
    }

    let where_predicates = ast.generics.where_clause.as_ref().map(|w| &w.predicates);

    let constraints = TokenStream::from_iter(extract_field_types(&ast.data).iter().map(|t| {
        quote! {
            #t: ossa_typeable::Typeable,
        }
    }));

    quote! {
        where
            #where_predicates
            #constraints
    }
}

fn impl_typeable_macro(ast: &syn::DeriveInput) -> proc_macro::TokenStream {
    let name = &ast.ident;
    let name_lit = ast.ident.to_string();
    let type_args_count: u8 = ast
        .generics
        .params
        .iter()
        .filter(|t| is_type_arg(t))
        .count()
        .try_into()
        .unwrap();
    let type_args = &ast.generics.params;
    let abbr_type_args = TokenStream::from_iter(ast.generics.params.iter().map(|p| match p {
        GenericParam::Lifetime(l) => {
            let name = &l.lifetime;
            quote! {
                #name,
            }
        }
        GenericParam::Type(t) => {
            let name = &t.ident;
            quote! {
                #name,
            }
        }
        GenericParam::Const(c) => {
            let name = &c.ident;
            quote! {
                #name,
            }
        }
    }));
    let body = match &ast.data {
        Data::Struct(s) => {
            let fs = impl_fields(&s.fields);
            quote! {
                helper_u8(&mut h, 0);
                #fs
            }
        }
        Data::Enum(e) => {
            let c = e.variants.len();
            let vs = TokenStream::from_iter(e.variants.iter().map(|v| {
                let name = v.ident.to_string();
                let fs = impl_fields(&v.fields);

                quote! {
                    helper_string(&mut h, #name);
                    #fs
                }
            }));
            quote! {
                helper_u8(&mut h, 1);
                helper_counter(#c);
                #vs
            }
        }
        Data::Union(_) => {
            panic!("Union types are not supported.")
        }
    };
    let where_clause = impl_where(&ast);
    let tag = impl_tag(&ast.attrs);

    let gen = quote! {
        #[automatically_derived]
        impl<#type_args> ossa_typeable::Typeable for #name<#abbr_type_args> #where_clause {
            fn type_ident() -> ossa_typeable::TypeId {
                use ossa_typeable::internal::*;

                let mut h = Sha256::new();
                helper_type_constructor(&mut h, #name_lit);
                helper_type_args_count(&mut h, #type_args_count);

                #tag
                #body

                ossa_typeable::TypeId::new(h.finalize().into())
            }
        }
    };
    gen.into()
}

fn is_type_arg(a: &GenericParam) -> bool {
    match a {
        GenericParam::Type(_) => true,
        _ => false,
    }
}

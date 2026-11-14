use proc_macro::TokenStream;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Data;
use syn::DataStruct;
use syn::DeriveInput;
use syn::Error;
use syn::ExprClosure;
use syn::Fields;
use syn::Ident;
use syn::LitBool;
use syn::Type;
use syn::parse_macro_input;

/// Log levels, one `log_<level>` and one `log_<level>_every` method each.
const LEVELS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];

/// Numeric types and the atomic type that holds each one.
const NUMERICS: [(&str, &str); 14] = [
    ("u8", "AtomicU8"),
    ("u16", "AtomicU16"),
    ("u32", "AtomicU32"),
    ("u64", "AtomicU64"),
    ("u128", "AtomicU128"),
    ("usize", "AtomicUsize"),
    ("i8", "AtomicI8"),
    ("i16", "AtomicI16"),
    ("i32", "AtomicI32"),
    ("i64", "AtomicI64"),
    ("i128", "AtomicI128"),
    ("isize", "AtomicIsize"),
    ("f32", "AtomicF32"),
    ("f64", "AtomicF64"),
];

/// Entry that `extra` adds after a field, computed from that field.
enum Extra {
    /// The field's `elapsed()` in whole units.
    Elapsed { unit: TimeUnit },
    /// The field's value divided by another field's `elapsed()` in units, rounded to one decimal.
    Rate { reference: Ident, unit: TimeUnit },
    /// The result of a closure that receives the field's value.
    Transform { function: ExprClosure },
}

/// Unit of time that `kind = elapsed` and `kind = rate` use.
enum TimeUnit {
    /// Milliseconds, written `ms`.
    Milliseconds,
    /// Seconds, written `s`.
    Seconds,
    /// Minutes, written `min`.
    Minutes,
    /// Hours, written `h`.
    Hours,
}

impl TimeUnit {
    /// Parses the unit written in `unit = <unit>`.
    fn parse(unit: &Ident) -> syn::Result<Self> {
        match unit.to_string().as_str() {
            "ms" => Ok(Self::Milliseconds),
            "s" => Ok(Self::Seconds),
            "min" => Ok(Self::Minutes),
            "h" => Ok(Self::Hours),
            _ => Err(Error::new_spanned(unit, "unit must be ms, s, min or h")),
        }
    }

    /// Returns the unit as written, used as the suffix of the default key.
    fn suffix(&self) -> &'static str {
        match self {
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Minutes => "min",
            Self::Hours => "h",
        }
    }

    /// Returns the expression that reads the field's `elapsed()` in whole units.
    fn read(&self, name: &Ident) -> proc_macro2::TokenStream {
        match self {
            Self::Milliseconds => quote!(self.#name.elapsed().as_millis()),
            Self::Seconds => quote!(self.#name.elapsed().as_secs()),
            Self::Minutes => quote!(self.#name.elapsed().as_secs() / 60),
            Self::Hours => quote!(self.#name.elapsed().as_secs() / 3600),
        }
    }

    /// Returns the expression that reads the field's `elapsed()` in units, with the fraction.
    fn read_fraction(&self, name: &Ident) -> proc_macro2::TokenStream {
        match self {
            Self::Milliseconds => quote!(self.#name.elapsed().as_secs_f64() * 1000.0),
            Self::Seconds => quote!(self.#name.elapsed().as_secs_f64()),
            Self::Minutes => quote!(self.#name.elapsed().as_secs_f64() / 60.0),
            Self::Hours => quote!(self.#name.elapsed().as_secs_f64() / 3600.0),
        }
    }
}

/// Generates a `log_<level>` method that logs the fields and a `log_<level>_every` method that repeats it while a closure runs, for each log level.
#[proc_macro_derive(SelfLog, attributes(selflog))]
pub fn derive_selflog(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // read named fields
    if let Some(attribute) = input.attrs.iter().find(|attribute| attribute.path().is_ident("selflog")) {
        return Error::new_spanned(attribute, "#[selflog] goes on fields; the log level is chosen by calling log_<level>").to_compile_error().into();
    }
    let Data::Struct(DataStruct { fields: Fields::Named(fields), .. }) = &input.data else {
        return Error::new_spanned(&input.ident, "SelfLog needs a struct with named fields").to_compile_error().into();
    };
    let field_names: Vec<&Ident> = fields.named.iter().filter_map(|field| field.ident.as_ref()).collect();

    // generate log entries
    let relaxed = quote!(::std::sync::atomic::Ordering::Relaxed);
    let mut log_keys = Vec::new();
    let mut log_values = Vec::new();
    let mut log_sigils = Vec::new();
    for field in &fields.named {
        let name = field.ident.as_ref().expect("named field");
        let mut entry_seen = false;
        let mut include: Option<bool> = None;
        let mut entry_key: Option<Ident> = None;
        let mut extras: Vec<(Ident, Extra)> = Vec::new();
        for attribute in field.attrs.iter().filter(|attribute| attribute.path().is_ident("selflog")) {
            let options = attribute.parse_nested_meta(|meta| {
                let option = meta.path.require_ident()?.to_string();
                match option.as_str() {
                    "entry" => {
                        if entry_seen {
                            return Err(meta.error("entry appears twice on this field"));
                        }
                        entry_seen = true;
                        if !meta.input.peek(syn::token::Paren) {
                            return Err(meta.error("entry needs an argument: entry(include = true), entry(include = false) or entry(key = <key>)"));
                        }
                        meta.parse_nested_meta(|argument| {
                            if argument.path.is_ident("include") {
                                include = Some(argument.value()?.parse::<LitBool>()?.value);
                                return Ok(());
                            }
                            if argument.path.is_ident("key") {
                                entry_key = Some(argument.value()?.parse()?);
                                return Ok(());
                            }
                            Err(argument.error("entry arguments are include and key"))
                        })
                    }
                    "extra" => {
                        let mut kind: Option<Ident> = None;
                        let mut key: Option<Ident> = None;
                        let mut reference: Option<Ident> = None;
                        let mut function: Option<ExprClosure> = None;
                        let mut unit: Option<Ident> = None;
                        if meta.input.peek(syn::token::Paren) {
                            meta.parse_nested_meta(|argument| {
                                if argument.path.is_ident("kind") {
                                    kind = Some(argument.value()?.parse()?);
                                    return Ok(());
                                }
                                if argument.path.is_ident("key") {
                                    key = Some(argument.value()?.parse()?);
                                    return Ok(());
                                }
                                if argument.path.is_ident("reference") {
                                    reference = Some(argument.value()?.parse()?);
                                    return Ok(());
                                }
                                if argument.path.is_ident("value") {
                                    function = Some(argument.value()?.parse().map_err(|error| Error::new(error.span(), "value must be a closure: |<name>| <expression>"))?);
                                    return Ok(());
                                }
                                if argument.path.is_ident("unit") {
                                    unit = Some(argument.value()?.parse()?);
                                    return Ok(());
                                }
                                Err(argument.error("extra arguments are kind, key, reference, value and unit"))
                            })?;
                        }
                        let Some(kind) = kind else { return Err(meta.error("extra needs kind: extra(kind = elapsed | rate | transform, ...)")) };
                        let extra = match kind.to_string().as_str() {
                            "elapsed" if reference.is_some() || function.is_some() => return Err(Error::new_spanned(&kind, "kind = elapsed takes unit and key")),
                            "elapsed" => {
                                let Some(unit) = unit else { return Err(Error::new_spanned(&kind, "kind = elapsed needs unit = ms | s | min | h")) };
                                Extra::Elapsed { unit: TimeUnit::parse(&unit)? }
                            }
                            "rate" if function.is_some() => return Err(Error::new_spanned(&kind, "kind = rate takes reference, unit and key")),
                            "rate" => {
                                let Some(reference) = reference else { return Err(Error::new_spanned(&kind, "kind = rate needs reference = <field>")) };
                                let Some(unit) = unit else { return Err(Error::new_spanned(&kind, "kind = rate needs unit = ms | s | min | h")) };
                                Extra::Rate { reference, unit: TimeUnit::parse(&unit)? }
                            }
                            "transform" if reference.is_some() || unit.is_some() => return Err(Error::new_spanned(&kind, "kind = transform takes key and value")),
                            "transform" => {
                                let Some(function) = function else { return Err(Error::new_spanned(&kind, "kind = transform needs value = |<name>| <expression>")) };
                                Extra::Transform { function }
                            }
                            _ => return Err(Error::new_spanned(&kind, "kind must be elapsed, rate or transform")),
                        };
                        let key = match (key, &extra) {
                            (Some(key), _) => key,
                            (None, Extra::Elapsed { unit }) => format_ident!("{name}_elapsed_{}", unit.suffix(), span = name.span()),
                            (None, Extra::Rate { unit, .. }) => format_ident!("{name}_per_{}", unit.suffix(), span = name.span()),
                            (None, Extra::Transform { .. }) => return Err(Error::new_spanned(&kind, "kind = transform needs key = <key>")),
                        };
                        extras.push((key, extra));
                        Ok(())
                    }
                    _ => Err(meta.error("field option must be entry or extra")),
                }
            });
            if let Err(error) = options {
                return error.to_compile_error().into();
            }
        }
        if entry_seen && include.is_none() && entry_key.is_none() {
            return Error::new_spanned(name, "entry needs include or key").to_compile_error().into();
        }
        if let Some(reference) = extras.iter().find_map(|(_, extra)| match extra {
            Extra::Rate { reference, .. } if !field_names.contains(&reference) => Some(reference),
            _ => None,
        }) {
            return Error::new_spanned(reference, "reference must name a field of the struct").to_compile_error().into();
        }

        let type_name = match &field.ty {
            Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.to_string()),
            _ => None,
        };
        let numeric = type_name.as_deref().and_then(|type_name| NUMERICS.iter().find(|(value_type, atomic_type)| type_name == *value_type || type_name == *atomic_type));
        let number = numeric.map(|(value_type, atomic_type)| {
            let read = if type_name.as_deref() == Some(*atomic_type) { quote!(self.#name.load(#relaxed)) } else { quote!(self.#name) };
            (read, *value_type)
        });

        let included = include.unwrap_or(true);
        if entry_key.is_some() && !included {
            return Error::new_spanned(name, "an entry that is not logged has no key; set include = true or remove key").to_compile_error().into();
        }
        if included {
            log_keys.push(entry_key.unwrap_or_else(|| name.clone()));
            match &number {
                Some((read, _)) => {
                    log_values.push(read.clone());
                    log_sigils.push(quote!());
                }
                None => {
                    log_values.push(quote_spanned!(name.span()=> (&self.#name as &dyn ::std::fmt::Display)));
                    log_sigils.push(quote!(%));
                }
            }
        }
        for (key, extra) in extras {
            let value = match extra {
                Extra::Elapsed { unit } => unit.read(name),
                Extra::Rate { reference, unit } => {
                    let Some((read, value_type)) = &number else {
                        return Error::new_spanned(name, "kind = rate needs a numeric or atomic field").to_compile_error().into();
                    };
                    let count = if *value_type == "f64" { read.clone() } else { quote!(#read as f64) };
                    let elapsed = unit.read_fraction(&reference);
                    quote! {{
                        let elapsed = #elapsed;
                        if elapsed > 0.0 { (#count / elapsed * 10.0).round() / 10.0 } else { 0.0 }
                    }}
                }
                Extra::Transform { function } => {
                    let argument = match &number {
                        Some((read, _)) => read.clone(),
                        None => quote!(&self.#name),
                    };
                    quote! {{
                        fn transform<T, R>(value: T, function: impl FnOnce(T) -> R) -> R { function(value) }
                        transform(#argument, #function)
                    }}
                }
            };
            log_keys.push(key);
            log_values.push(value);
            log_sigils.push(quote!());
        }
    }

    // check log keys
    for (index, key) in log_keys.iter().enumerate() {
        if log_keys[..index].contains(key) {
            return Error::new_spanned(key, format!("another entry already logs {key}; set a different key")).to_compile_error().into();
        }
    }

    // generate log methods
    let vis = &input.vis;
    let format = log_keys.iter().fold(String::from("{}"), |format, key| format + &format!(" {key}={{}}"));
    let every_body = |log: proc_macro2::TokenStream| {
        quote! {
            struct Progress(::std::sync::mpsc::Sender<()>);
            impl Drop for Progress {
                fn drop(&mut self) {}
            }

            let (sender, receiver) = ::std::sync::mpsc::channel::<()>();
            scope.spawn(move || {
                while let Err(::std::sync::mpsc::RecvTimeoutError::Timeout) = receiver.recv_timeout(interval) {
                    #log;
                }
            });
            Progress(sender)
        }
    };
    let logs = LEVELS.map(|level| {
        let method = format_ident!("log_{level}");
        let every = format_ident!("log_{level}_every");
        let doc = format!("Logs every field at {level} level.");
        let every_doc = format!("Logs every field at {level} level each interval on a thread of the scope, until the returned value is dropped.");
        let body = if cfg!(feature = "tracing") {
            let level = format_ident!("{level}");
            quote!(::tracing::#level!(#(#log_keys = #log_sigils #log_values,)* "{}", message);)
        } else {
            quote!(println!(#format, message #(, #log_values)*);)
        };
        let every_body = every_body(quote!(self.#method(message)));
        quote! {
            #[doc = #doc]
            #vis fn #method(&self, message: &str) {
                #body
            }

            #[doc = #every_doc]
            #[must_use = "logging stops when the returned value is dropped"]
            #vis fn #every<'scope, 'env>(&'env self, scope: &'scope ::std::thread::Scope<'scope, 'env>, interval: ::std::time::Duration, message: &'env str) -> impl Drop
            where
                for<'a> &'a Self: Send,
            {
                #every_body
            }
        }
    });

    // generate log methods with level
    let leveled = cfg!(feature = "tracing").then(|| {
        let every_body = every_body(quote!(self.log(level, message)));
        quote! {
            /// Logs every field at the level.
            #vis fn log(&self, level: ::tracing::Level, message: &str) {
                match level {
                    ::tracing::Level::TRACE => self.log_trace(message),
                    ::tracing::Level::DEBUG => self.log_debug(message),
                    ::tracing::Level::INFO => self.log_info(message),
                    ::tracing::Level::WARN => self.log_warn(message),
                    _ => self.log_error(message),
                }
            }

            /// Logs every field at the level each interval on a thread of the scope, until the returned value is dropped.
            #[must_use = "logging stops when the returned value is dropped"]
            #vis fn log_every<'scope, 'env>(&'env self, level: ::tracing::Level, scope: &'scope ::std::thread::Scope<'scope, 'env>, interval: ::std::time::Duration, message: &'env str) -> impl Drop
            where
                for<'a> &'a Self: Send,
            {
                #every_body
            }
        }
    });

    // emit methods
    let ident = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    quote! {
        #[allow(dead_code)]
        impl #impl_generics #ident #type_generics #where_clause {
            #(#logs)*

            #leveled
        }
    }
    .into()
}

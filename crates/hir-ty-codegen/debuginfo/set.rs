//! The sets of types a debugger is told of, kept so that the sets with a type share what it
//! is told as. A set is a list of [`DebugType`]s that name each other by their places in it,
//! and the same type is in many: every type a variable's type is made of is in its set, and
//! every set of a type with a `String` in it has the `String`, its `Vec`, its `RawVec` and
//! the rest. What a type is told as apart from those places ([`Told`]) is interned, and a set
//! has a reference to it and the places of the types it is made of, one number each.

use std::{fmt, sync::Arc};

use intern::{Interned, Symbol, impl_internable};

use super::{Aggregate, DebugType, Field, Tag, TypeIndex, Variant};

/// A type and every type it is made of, the type itself first. The functions with a
/// variable of the type share the one set.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TypeSet(Arc<Types>);

#[derive(PartialEq, Eq)]
struct Types {
    told: Box<[Interned<Told>]>,
    /// Where the places of the types each one is made of start in `of`, and after the last
    /// one where its end.
    starts: Box<[u32]>,
    of: Box<[TypeIndex]>,
}

/// What a debugger is told of a type apart from where in its set the types it is made of are,
/// which the type is made of in the order [`TypeSet::get`] tells.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) enum Told {
    Base {
        name: Symbol,
        size: u64,
        encoding: u8,
    },
    /// Made of what it points to where that is told.
    Pointer {
        name: Symbol,
    },
    /// Made of its element.
    Array {
        len: u64,
    },
    /// Made of the types of its fields, then of what its generic parameters stand for.
    Struct(Shape),
    Union(Shape),
    Names {
        name: Symbol,
        size: u64,
        names: Box<[(Symbol, u64)]>,
    },
    /// Made of what [`Told::Struct`] is, then of the types of the fields of each variant.
    Enum {
        of: Shape,
        tag: Option<Tag>,
        variants: Box<[Shaped]>,
    },
}

impl_internable!(Told);

/// An [`Aggregate`] without the places of its types.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct Shape {
    pub(super) name: Symbol,
    pub(super) size: u64,
    /// The name and the offset of each field.
    pub(super) fields: Box<[(Symbol, u64)]>,
    pub(super) parameters: Box<[Symbol]>,
}

/// A [`Variant`] without the places of the types of its fields.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct Shaped {
    pub(super) name: Symbol,
    pub(super) tag: Option<u64>,
    pub(super) fields: Box<[(Symbol, u64)]>,
}

impl TypeSet {
    pub(crate) fn len(&self) -> usize {
        self.0.told.len()
    }

    /// What tells the set from every other one there is while it is.
    pub(crate) fn at(&self) -> usize {
        Arc::as_ptr(&self.0).addr()
    }

    /// The type at `index`, and the places of the types it is made of.
    pub(super) fn get(&self, index: TypeIndex) -> Option<(&Told, &[TypeIndex])> {
        let Types { told, starts, of } = &*self.0;
        let index = index as usize;
        let told = told.get(index)?;
        Some((told, &of[starts[index] as usize..starts[index + 1] as usize]))
    }

    /// The set as a list of types, which is how the store keeps it.
    pub(crate) fn to_vec(&self) -> Vec<DebugType> {
        (0..self.len() as TypeIndex).filter_map(|index| self.get(index)).map(expand).collect()
    }
}

impl From<&[DebugType]> for TypeSet {
    fn from(types: &[DebugType]) -> Self {
        let mut starts = Vec::with_capacity(types.len() + 1);
        let mut of = Vec::new();
        let told = types
            .iter()
            .map(|ty| {
                starts.push(of.len() as u32);
                Interned::new(shape(ty, &mut of))
            })
            .collect();
        starts.push(of.len() as u32);
        TypeSet(Arc::new(Types { told, starts: starts.into(), of: of.into() }))
    }
}

impl From<Vec<DebugType>> for TypeSet {
    fn from(types: Vec<DebugType>) -> Self {
        TypeSet::from(types.as_slice())
    }
}

impl fmt::Debug for TypeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.to_vec()).finish()
    }
}

/// What `ty` is told as, with the places of the types it is made of pushed to `of`.
fn shape(ty: &DebugType, of: &mut Vec<TypeIndex>) -> Told {
    let names = |fields: &[Field], of: &mut Vec<TypeIndex>| -> Box<[(Symbol, u64)]> {
        of.extend(fields.iter().map(|it| it.ty));
        fields.iter().map(|it| (Symbol::intern(&it.name), it.offset)).collect()
    };
    let aggregate = |it: &Aggregate, of: &mut Vec<TypeIndex>| {
        let fields = names(&it.fields, of);
        of.extend(it.parameters.iter().map(|(_, ty)| *ty));
        let parameters = it.parameters.iter().map(|(name, _)| Symbol::intern(name)).collect();
        Shape { name: Symbol::intern(&it.name), size: it.size, fields, parameters }
    };
    match ty {
        DebugType::Base { name, size, encoding } => {
            Told::Base { name: Symbol::intern(name), size: *size, encoding: *encoding }
        }
        DebugType::Pointer { name, to } => {
            of.extend(*to);
            Told::Pointer { name: Symbol::intern(name) }
        }
        DebugType::Array { of: element, len } => {
            of.push(*element);
            Told::Array { len: *len }
        }
        DebugType::Struct(it) => Told::Struct(aggregate(it, of)),
        DebugType::Union(it) => Told::Union(aggregate(it, of)),
        DebugType::Names { name, size, names } => Told::Names {
            name: Symbol::intern(name),
            size: *size,
            names: names.iter().map(|(name, value)| (Symbol::intern(name), *value)).collect(),
        },
        DebugType::Enum { of: it, tag, variants } => {
            let it = aggregate(it, of);
            let variants = variants
                .iter()
                .map(|it| Shaped {
                    name: Symbol::intern(&it.name),
                    tag: it.tag,
                    fields: names(&it.fields, of),
                })
                .collect();
            Told::Enum { of: it, tag: *tag, variants }
        }
    }
}

/// The type `told` is, made of the types at `of`.
fn expand((told, of): (&Told, &[TypeIndex])) -> DebugType {
    type Places<'a> = std::iter::Copied<std::slice::Iter<'a, TypeIndex>>;
    fn fields(names: &[(Symbol, u64)], of: &mut Places<'_>) -> Vec<Field> {
        let fields = names.iter().zip(of);
        fields
            .map(|((name, offset), ty)| Field { name: name.to_string(), offset: *offset, ty })
            .collect()
    }
    fn aggregate(it: &Shape, of: &mut Places<'_>) -> Aggregate {
        let fields = fields(&it.fields, of);
        let parameters = it.parameters.iter().zip(of).map(|(name, ty)| (name.to_string(), ty));
        Aggregate {
            name: it.name.to_string(),
            size: it.size,
            fields,
            parameters: parameters.collect(),
        }
    }
    let mut of = of.iter().copied();
    match told {
        Told::Base { name, size, encoding } => {
            DebugType::Base { name: name.to_string(), size: *size, encoding: *encoding }
        }
        Told::Pointer { name } => DebugType::Pointer { name: name.to_string(), to: of.next() },
        Told::Array { len } => DebugType::Array { of: of.next().unwrap_or_default(), len: *len },
        Told::Struct(it) => DebugType::Struct(aggregate(it, &mut of)),
        Told::Union(it) => DebugType::Union(aggregate(it, &mut of)),
        Told::Names { name, size, names } => DebugType::Names {
            name: name.to_string(),
            size: *size,
            names: names.iter().map(|(name, value)| (name.to_string(), *value)).collect(),
        },
        Told::Enum { of: it, tag, variants } => {
            let it = aggregate(it, &mut of);
            let variants = variants.iter().map(|variant| {
                let fields = fields(&variant.fields, &mut of);
                Variant { name: variant.name.to_string(), tag: variant.tag, fields }
            });
            DebugType::Enum { of: it, tag: *tag, variants: variants.collect() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, offset: u64, ty: TypeIndex) -> Field {
        Field { name: name.into(), offset, ty }
    }

    /// `Option<Box<List>>` where a `List` holds a `u8` and the `Option` of the next one: a
    /// pointer back into the set, an enum with a tag, a generic parameter and an array.
    fn list() -> Vec<DebugType> {
        let aggregate = |name: &str, size, fields, parameters| Aggregate {
            name: name.into(),
            size,
            fields,
            parameters,
        };
        vec![
            DebugType::Enum {
                of: aggregate("Option<Box<List>>", 8, vec![], vec![("T".into(), 1)]),
                tag: Some(Tag { offset: 0, size: 8, states: false }),
                variants: vec![
                    Variant { name: "None".into(), tag: Some(0), fields: vec![] },
                    Variant { name: "Some".into(), tag: None, fields: vec![field("__0", 0, 1)] },
                ],
            },
            DebugType::Pointer { name: "Box<List>".into(), to: Some(2) },
            DebugType::Struct(aggregate(
                "List",
                24,
                vec![field("value", 8, 3), field("next", 0, 0), field("pad", 9, 4)],
                vec![],
            )),
            DebugType::Base { name: "u8".into(), size: 1, encoding: 8 },
            DebugType::Array { of: 3, len: 7 },
            DebugType::Pointer { name: "*const ()".into(), to: None },
            DebugType::Names {
                name: "Ordering".into(),
                size: 1,
                names: vec![("Less".into(), 255), ("Equal".into(), 0)],
            },
            DebugType::Union(aggregate(
                "MaybeUninit<u8>",
                1,
                vec![field("value", 0, 3)],
                vec![("T".into(), 3)],
            )),
        ]
    }

    #[test]
    fn a_set_is_what_it_was_made_of() {
        let types = list();
        let set = TypeSet::from(types.as_slice());
        assert_eq!(set.to_vec(), types);
    }

    #[test]
    fn sets_with_a_type_share_what_it_is_told_as() {
        let one = TypeSet::from(list());
        let mut types = list();
        types.swap(3, 5);
        let other = TypeSet::from(types);
        let told = |set: &TypeSet, index: usize| -> *const Told { &*set.0.told[index] };
        assert_eq!(told(&one, 2), told(&other, 2));
        assert_eq!(told(&one, 3), told(&other, 5));
        assert_ne!(one, other);
    }
}

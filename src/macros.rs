macro_rules! columns_enum {
    (
        pub(crate) enum $ident:ident {
            $($variant:ident),* $(,)?
        }
    ) => {
        pub(crate) enum $ident {
            $($variant),*
        }

        impl $ident {
            fn all() -> Vec<Self> {
                Vec::from([ $(Self::$variant),* ])
            }
        }

        impl std::fmt::Display for $ident {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    $(
                        Self::$variant => write!(f, stringify!($variant)),
                    )*
                }
            }
        }
    };
}

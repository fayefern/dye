use dye_assets::source::PackSource;

pub fn source() -> Box<dyn PackSource> {
    #[cfg(debug_assertions)]
    {
        use dye_assets::source::DirSource;

        Box::new(DirSource::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/builtin/core"
        )))
    }

    #[cfg(not(debug_assertions))]
    {
        use dye_assets::source::EmbeddedSource;
        use include_dir::{Dir, include_dir};

        static DYE_RES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/builtin/core");

        Box::new(EmbeddedSource::new("<builtin>", &DYE_RES))
    }
}

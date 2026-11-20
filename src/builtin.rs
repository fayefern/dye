use dye_assets::source::PackSource;

pub fn source() -> Box<dyn PackSource> {
    #[cfg(debug_assertions)]
    {
        use dye_assets::source::DirSource;

        Box::new(DirSource::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/builtin/packs"
        )))
    }

    #[cfg(not(debug_assertions))]
    {
        use dye_assets::source::EmbeddedSource;
        use include_dir::{Dir, include_dir};

        static DYE_ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/builtin/packs");

        Box::new(EmbeddedSource::new("<builtin>", &DYE_ASSETS))
    }
}

use dye_res::source::ResSource;

pub fn source() -> Box<dyn ResSource> {
    #[cfg(debug_assertions)]
    {
        use dye_res::source::DirSource;

        Box::new(DirSource::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/builtin/dye"
        )))
    }

    #[cfg(not(debug_assertions))]
    {
        use dye_res::source::EmbeddedSource;
        use include_dir::{Dir, include_dir};

        static DYE_RES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/builtin/dye");

        Box::new(EmbeddedSource::new("<builtin>", &DYE_RES))
    }
}

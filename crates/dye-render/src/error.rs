use thiserror::Error;

#[derive(Error, Debug)]
#[error("{0}")]
pub struct BackendError(pub String);

#[derive(Error, Debug)]
pub enum AtlasError {
    #[error("texture `{id}` is {w}*{h} (+padding), which exceeds the atlas limit of {max}")]
    TileTooLarge {
        id: String,
        w: u32,
        h: u32,
        max: u32,
    },
    #[error("textures do not fit in a {max}*{max} atlas")]
    TooLarge { max: u32 },
}

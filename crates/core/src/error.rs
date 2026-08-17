use thiserror::Error;

use crate::id::ObjectId;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ContainerError {
    #[error("object {0:?} not found")]
    ObjectNotFound(ObjectId),

    #[error("object {id:?} has wrong type, expected {expected}")]
    WrongObjectType {
        id: ObjectId,
        expected: &'static str,
    },

    #[error("variable {0:?} not found")]
    VariableNotFound(String),
}

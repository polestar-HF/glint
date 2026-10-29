//! Stand-in used when neither `ort-dynamic` nor `ort-static` is enabled: the crate still builds
//! (so the workspace can be checked without an ONNX Runtime), but `PaddleOcr::load` fails.

use std::path::Path;

use super::{rec::Dict, PaddleOptions};
use crate::{OcrError, Result};

pub(super) struct Engine {
    pub dict: Dict,
}

pub(super) fn default_threads() -> usize {
    1
}

impl Engine {
    pub fn load(_model_dir: &Path, _opts: &PaddleOptions) -> Result<Self> {
        Err(OcrError::Unsupported)
    }

    pub fn run_det(&self, _data: Vec<f32>, _h: usize, _w: usize) -> Result<Vec<f32>> {
        Err(OcrError::Unsupported)
    }

    pub fn run_rec<R>(
        &self,
        _data: Vec<f32>,
        _n: usize,
        _w: usize,
        _f: impl FnOnce(&[f32], usize, usize, usize) -> R,
    ) -> Result<R> {
        Err(OcrError::Unsupported)
    }

    pub fn run_cls(&self, _data: Vec<f32>, _n: usize) -> Result<Vec<[f32; 2]>> {
        Err(OcrError::Unsupported)
    }

    pub fn has_cls(&self) -> bool {
        false
    }
}

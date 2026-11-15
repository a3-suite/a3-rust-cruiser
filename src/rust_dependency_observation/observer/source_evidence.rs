use std::path::Path;

use proc_macro2::LineColumn;

use super::Observer;
use crate::boundary_validation::SourceEvidence;

impl Observer {
    pub(super) fn evidence(&self, path: &Path, start: LineColumn) -> SourceEvidence {
        let relative = self.relative_path(path);
        let column = self
            .source_lines
            .get(path)
            .and_then(|lines| lines.get(start.line.saturating_sub(1)))
            .map(|line| {
                line.char_indices()
                    .take_while(|(offset, _)| *offset < start.column)
                    .count()
                    + 1
            })
            .unwrap_or(start.column + 1);
        SourceEvidence {
            path: relative,
            line: start.line,
            column,
        }
    }

    pub(super) fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }
}

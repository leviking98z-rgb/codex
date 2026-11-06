//! Display-only alias integration; saved thread titles remain independently editable.

use super::*;

impl ChatWidget {
    pub(super) fn display_thread_name(&self) -> Option<String> {
        self.session_alias
            .name()
            .or_else(|| self.thread_name.clone())
    }

    pub(crate) fn on_session_alias_updated(&mut self, thread_id: ThreadId) {
        if self.thread_id == Some(thread_id) {
            self.refresh_status_surfaces();
            self.request_redraw();
        }
    }
}

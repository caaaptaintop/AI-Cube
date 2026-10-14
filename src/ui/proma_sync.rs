use super::*;

impl KitterApp {
    pub(super) fn start_local_proma_sync(&mut self, cx: &mut Context<Self>) {
        let Some(root) = crate::proma::home() else {
            return;
        };
        self.model.proma_sync_task = Some(cx.spawn(async move |this, cx| {
            let mut sync = crate::proma::LocalMirrorSync::default();
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let Ok(sources) =
                    this.update(cx, |this, _| this.model.library.local_source_paths())
                else {
                    break;
                };
                let root = root.clone();
                let (returned_sync, report) = cx
                    .background_executor()
                    .spawn(async move {
                        let report = sync.poll(&root, &sources).unwrap_or_else(|error| {
                            crate::proma::LocalSyncReport {
                                warnings: vec![format!("{error}")],
                                ..Default::default()
                            }
                        });
                        (sync, report)
                    })
                    .await;
                sync = returned_sync;
                if this
                    .update(cx, |this, cx| {
                        let changed =
                            report.updated > 0 || this.model.proma_sync_warnings != report.warnings;
                        this.model.proma_sync_warnings = report.warnings;
                        if changed {
                            this.refresh(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}

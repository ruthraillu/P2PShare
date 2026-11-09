//! Owns the optional local capture independently of the stream publisher.
use p2pshare_media::{CaptureBackend, CaptureSource, CaptureState};

pub(crate) trait PreviewBackend: CaptureBackend {
    fn state_for_preview(&self) -> CaptureState;
    fn set_cursor_capture(&mut self,_enabled:bool)->Result<(),Self::Error>{Ok(())}
}
impl PreviewBackend for p2pshare_platform_windows::WindowsCapture {
    fn set_cursor_capture(&mut self,enabled:bool)->Result<(),Self::Error>{p2pshare_platform_windows::WindowsCapture::set_cursor_capture(self,enabled)}
    fn state_for_preview(&self) -> CaptureState {
        self.state()
    }
}

pub(crate) struct LocalPreview<B: PreviewBackend> {
    backend: B,
    source: Option<CaptureSource>,
    visible: bool,
}
impl<B: PreviewBackend> LocalPreview<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            source: None,
            visible: true,
        }
    }
    pub fn start(&mut self, source: &CaptureSource) -> Result<(), B::Error> {
        self.stop()?;
        if self.visible {
            self.backend.start(source)?;
        }
        self.source = Some(source.clone());
        Ok(())
    }
    pub fn set_cursor_capture(&mut self,enabled:bool)->Result<(),B::Error>{self.backend.set_cursor_capture(enabled)}
    pub fn set_visible(&mut self, visible: bool) -> Result<(), B::Error> {
        if self.visible == visible {
            return Ok(());
        }
        self.visible = visible;
        if !visible {
            self.backend.stop()?;
        } else if let Some(source) = &self.source {
            self.backend.start(source)?;
        }
        Ok(())
    }
    pub fn state(&self) -> CaptureState {
        self.backend.state_for_preview()
    }
    pub fn poll_frame(&mut self) -> Result<Option<B::Frame>, B::Error> {
        if self.visible {
            self.backend.poll_frame()
        } else {
            Ok(None)
        }
    }
    pub fn stop(&mut self) -> Result<(), B::Error> {
        self.source = None;
        self.backend.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2pshare_media::{Dimensions, SourceKind};
    #[derive(Default)]
    struct Fake {
        starts: usize,
        stops: usize,
        active: bool,
        polls: usize,
        cursor:bool,
    }
    impl CaptureBackend for Fake {
        type Frame = ();
        type Error = std::convert::Infallible;
        fn sources(&self) -> Result<Vec<CaptureSource>, Self::Error> {
            Ok(Vec::new())
        }
        fn start(&mut self, _: &CaptureSource) -> Result<(), Self::Error> {
            self.starts += 1;
            self.active = true;
            Ok(())
        }
        fn poll_frame(&mut self) -> Result<Option<()>, Self::Error> {
            self.polls += 1;
            Ok(None)
        }
        fn stop(&mut self) -> Result<(), Self::Error> {
            if self.active {
                self.stops += 1;
            }
            self.active = false;
            Ok(())
        }
    }
    impl PreviewBackend for Fake {
        fn set_cursor_capture(&mut self,enabled:bool)->Result<(),Self::Error>{self.cursor=enabled;Ok(())}
        fn state_for_preview(&self) -> CaptureState {
            if self.active {
                CaptureState::Capturing
            } else {
                CaptureState::Stopped
            }
        }
    }
    fn source(id: u64) -> CaptureSource {
        CaptureSource {
            id,
            kind: SourceKind::Window,
            title: "Owned fake".into(),
            size: Dimensions::new(640, 360),
            process_id: Some(1),
        }
    }
    #[test]
    fn hiding_releases_capture_showing_restarts_once_and_stop_cannot_restart() {
        let mut preview = LocalPreview::new(Fake::default());
        preview.start(&source(1)).unwrap();
        assert_eq!(preview.backend.starts, 1);
        preview.set_visible(false).unwrap();
        assert_eq!(preview.backend.stops, 1);
        for _ in 0..10 {
            preview.set_visible(false).unwrap();
            preview.poll_frame().unwrap();
        }
        assert_eq!(preview.backend.polls, 0);
        assert!(!preview.backend.active);
        preview.set_visible(true).unwrap();
        preview.set_visible(true).unwrap();
        assert_eq!(preview.backend.starts, 2);
        assert!(preview.backend.active);
        preview.stop().unwrap();
        preview.set_visible(false).unwrap();
        preview.set_visible(true).unwrap();
        assert_eq!(preview.backend.starts, 2);
        assert!(!preview.backend.active);
    }
    #[test]
    fn sharing_restarts_and_source_changes_while_hidden_do_not_start_capture() {
        let mut preview = LocalPreview::new(Fake::default());
        preview.set_visible(false).unwrap();
        preview.start(&source(1)).unwrap();
        preview.start(&source(2)).unwrap();
        assert_eq!(preview.backend.starts, 0);
        assert_eq!(preview.source.as_ref().unwrap().id, 2);
        preview.set_visible(true).unwrap();
        assert_eq!(preview.backend.starts, 1);
        preview.set_visible(false).unwrap();
        preview.stop().unwrap();
        preview.start(&source(3)).unwrap();
        assert_eq!(preview.backend.starts, 1);
        preview.set_visible(true).unwrap();
        assert_eq!(preview.backend.starts, 2);
    }
    #[test]
    fn cursor_changes_apply_without_restarting_visible_or_hidden_preview(){
        let mut preview=LocalPreview::new(Fake::default());preview.start(&source(1)).unwrap();
        preview.set_cursor_capture(false).unwrap();preview.set_cursor_capture(true).unwrap();
        assert_eq!(preview.backend.starts,1);assert_eq!(preview.backend.stops,0);assert!(preview.backend.cursor);
        preview.set_visible(false).unwrap();preview.set_cursor_capture(false).unwrap();assert_eq!(preview.backend.starts,1);
        preview.set_visible(true).unwrap();assert_eq!(preview.backend.starts,2);assert!(!preview.backend.cursor);
    }

}

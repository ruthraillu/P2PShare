//! Non-blocking single-session DXGI shared texture bridge from Publisher to the GUI.
use p2pshare_media::Dimensions;
use p2pshare_platform_windows::GraphicsDevice;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use windows::{
    Win32::{
        Foundation::HANDLE,
        Graphics::{
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
    core::Interface,
};

const RING_SIZE: usize = 3;

struct SharedPool {
    textures: [ID3D11Texture2D; RING_SIZE],
    handles: [isize; RING_SIZE],
    size: Dimensions,
    generation: u64,
}

#[derive(Default)]
struct SharedPreviewState {
    pool: Option<Arc<SharedPool>>,
    generation: u64,
    sequence: u64,
    latest_slot: usize,
}

pub(crate) struct SharedPreview {
    enabled: AtomicBool,
    next_generation: AtomicU64,
    state: Mutex<SharedPreviewState>,
}

impl SharedPreview {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            next_generation: AtomicU64::new(1),
            state: Mutex::new(SharedPreviewState::default()),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn set_enabled(&self, enabled: bool) {
        let was_enabled = self.enabled.swap(enabled, Ordering::AcqRel);
        if was_enabled && !enabled {
            self.clear_pool();
        }
    }

    fn clear_pool(&self) {
        if let Ok(mut guard) = self.state.try_lock() {
            guard.pool = None;
            guard.sequence += 1;
        }
    }

    fn next_generation(&self) -> u64 {
        self.next_generation.fetch_add(1, Ordering::AcqRel)
    }

    fn take_for_receiver(&self, last_sequence: u64) -> Option<(Option<(Arc<SharedPool>, usize, u64)>, u64)> {
        let guard = self.state.lock().unwrap();
        let Some(pool) = guard.pool.as_ref() else {
            return Some((None, guard.sequence));
        };
        if guard.sequence == last_sequence {
            return None;
        }
        Some((Some((pool.clone(), guard.latest_slot, guard.generation)), guard.sequence))
    }
}

pub(crate) struct SharedPreviewProducer {
    shared: Arc<SharedPreview>,
    pool: Option<Arc<SharedPool>>,
    slot: usize,
}

impl SharedPreviewProducer {
    pub fn new(shared: Arc<SharedPreview>) -> Self {
        Self {
            shared,
            pool: None,
            slot: 0,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.shared.is_enabled()
    }

    pub fn clear(&mut self) {
        if self.pool.take().is_some() {
            let mut guard = self.shared.state.lock().unwrap();
            guard.pool = None;
            guard.sequence += 1;
        }
    }

    pub fn publish(&mut self, graphics: &GraphicsDevice, source: &ID3D11Texture2D, size: Dimensions) {
        if !self.shared.is_enabled() || size.is_empty() {
            self.clear();
            return;
        }
        if self.pool.as_ref().is_none_or(|p| p.size != size) {
            match create_shared_pool(graphics, size, self.shared.next_generation()) {
                Ok(pool) => {
                    self.pool = Some(pool);
                    self.slot = 0;
                }
                Err(error) => {
                    tracing::warn!(%error, width = size.width, height = size.height, "shared_preview_pool_failed");
                    self.clear();
                    return;
                }
            }
        }
        let Some(pool) = &self.pool else {
            return;
        };
        let next_slot = (self.slot + 1) % RING_SIZE;
        // SAFETY: both textures belong to `graphics.device` and match `size` / BGRA8 format.
        unsafe {
            graphics.context.CopyResource(&pool.textures[next_slot], source);
            graphics.context.Flush();
        }
        if let Ok(mut guard) = self.shared.state.try_lock() {
            self.slot = next_slot;
            guard.generation = pool.generation;
            guard.latest_slot = next_slot;
            guard.sequence += 1;
            guard.pool = Some(pool.clone());
        }
    }
}

fn create_shared_pool(
    graphics: &GraphicsDevice,
    size: Dimensions,
    generation: u64,
) -> windows::core::Result<Arc<SharedPool>> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
    };
    let mut textures: Vec<ID3D11Texture2D> = Vec::with_capacity(RING_SIZE);
    let mut handles = [0isize; RING_SIZE];
    for slot in handles.iter_mut() {
        // SAFETY: `desc` describes a valid BGRA8 shared render-target/shader-resource texture.
        unsafe {
            let mut texture = None;
            graphics.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.unwrap();
            let resource: IDXGIResource = texture.cast()?;
            let handle = resource.GetSharedHandle()?;
            *slot = handle.0 as isize;
            textures.push(texture);
        }
    }
    let textures: [ID3D11Texture2D; RING_SIZE] = textures
        .try_into()
        .unwrap_or_else(|_| unreachable!("Fixed ring size"));
    Ok(Arc::new(SharedPool {
        textures,
        handles,
        size,
        generation,
    }))
}

#[derive(Default)]
pub(crate) struct SharedPreviewReceiver {
    generation: u64,
    last_sequence: u64,
    active_slot: Option<usize>,
    slots: [Option<(ID3D11Texture2D, egui::TextureId)>; RING_SIZE],
}

impl SharedPreviewReceiver {
    pub fn has_frame(&self) -> bool {
        self.active_slot
            .and_then(|slot| self.slots.get(slot))
            .is_some_and(Option::is_some)
    }

    pub fn clear(&mut self, renderer: &mut egui_directx11::Renderer) {
        self.active_slot = None;
        for entry in &mut self.slots {
            if let Some((_, id)) = entry.take() {
                renderer.unregister_user_texture(id);
            }
        }
    }

    pub fn poll(
        &mut self,
        graphics: &GraphicsDevice,
        renderer: &mut egui_directx11::Renderer,
        shared: &SharedPreview,
    ) -> Option<(egui::TextureId, Dimensions)> {
        let Some((frame, sequence)) = shared.take_for_receiver(self.last_sequence) else {
            return None;
        };
        let Some((pool, slot, generation)) = frame else {
            self.last_sequence = sequence;
            self.clear(renderer);
            return None;
        };
        if self.generation != generation {
            self.clear(renderer);
            self.generation = generation;
        }
        if self.slots[slot].is_none() {
            // SAFETY: `pool` stays alive via `Arc` across OpenSharedResource; handle is DXGI-owned.
            unsafe {
                let mut opened: Option<ID3D11Texture2D> = None;
                if let Err(error) = graphics
                    .device
                    .OpenSharedResource(HANDLE(pool.handles[slot] as *mut _), &mut opened)
                {
                    tracing::warn!(%error, slot, "shared_preview_open_failed");
                    self.last_sequence = sequence;
                    return None;
                }
                let Some(texture) = opened else {
                    self.last_sequence = sequence;
                    return None;
                };
                let mut srv = None;
                if let Err(error) = graphics
                    .device
                    .CreateShaderResourceView(&texture, None, Some(&mut srv))
                {
                    tracing::warn!(%error, slot, "shared_preview_srv_failed");
                    self.last_sequence = sequence;
                    return None;
                }
                let Some(srv) = srv else {
                    self.last_sequence = sequence;
                    return None;
                };
                let id = renderer.register_user_texture(srv);
                self.slots[slot] = Some((texture, id));
            }
        }
        self.last_sequence = sequence;
        self.active_slot = Some(slot);
        self.slots[slot].as_ref().map(|(_, id)| (*id, pool.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_source(graphics: &GraphicsDevice, size: Dimensions) -> ID3D11Texture2D {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.width,
            Height: size.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            ..Default::default()
        };
        let mut texture = None;
        unsafe {
            graphics
                .device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .unwrap();
        }
        texture.unwrap()
    }

    #[test]
    fn toggling_preview_releases_shared_state_and_never_blocks_on_lock() {
        let shared = SharedPreview::new();
        assert!(!shared.is_enabled());
        shared.set_enabled(true);
        assert!(shared.is_enabled());

        let graphics = GraphicsDevice::new().unwrap();
        let source = test_source(&graphics, Dimensions::new(128, 72));
        let mut producer = SharedPreviewProducer::new(shared.clone());

        // Holding the receiver mutex must never block the publisher thread.
        let held = shared.state.lock().unwrap();
        producer.publish(&graphics, &source, Dimensions::new(128, 72));
        drop(held);

        shared.set_enabled(false);
        assert!(!shared.is_enabled());
        producer.publish(&graphics, &source, Dimensions::new(128, 72));
        assert!(producer.pool.is_none());
        assert!(shared.take_for_receiver(0).is_some_and(|(frame, _)| frame.is_none()));
    }

    #[test]
    fn cross_device_shared_ring_publishes_and_clears_when_hardware_available() {
        let producer_gfx = GraphicsDevice::new().unwrap();
        let consumer_gfx = GraphicsDevice::new().unwrap();
        if !producer_gfx.hardware || !consumer_gfx.hardware {
            return;
        }
        let shared = SharedPreview::new();
        shared.set_enabled(true);
        let mut producer = SharedPreviewProducer::new(shared.clone());
        let mut renderer = egui_directx11::Renderer::new(&consumer_gfx.device).unwrap();
        let mut receiver = SharedPreviewReceiver::default();

        let source = test_source(&producer_gfx, Dimensions::new(160, 90));
        let mut first_ids = Vec::new();
        for _ in 0..=RING_SIZE {
            producer.publish(&producer_gfx, &source, Dimensions::new(160, 90));
            let (id, size) = receiver
                .poll(&consumer_gfx, &mut renderer, &shared)
                .expect("Shared preview frame missing");
            assert_eq!(size, Dimensions::new(160, 90));
            first_ids.push(id);
        }
        assert!(receiver.has_frame());
        assert_eq!(first_ids[0], first_ids[RING_SIZE]);

        producer.clear();
        assert!(receiver.poll(&consumer_gfx, &mut renderer, &shared).is_none());
        assert!(!receiver.has_frame());
    }
}

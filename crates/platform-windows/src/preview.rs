use crate::{GpuFrame, GraphicsDevice};
use p2pshare_media::{Dimensions, letterbox};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D::Fxc::*,
            Direct3D::*,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
    core::{Interface, Result, s},
};
const SHADER: &[u8] = br#"
struct V { float4 pos : SV_POSITION; float2 uv : TEXCOORD; };
V vs(uint id : SV_VertexID) {
    V o; float2 uv = float2((id << 1) & 2, id & 2);
    o.pos = float4(uv.x * 2 - 1, 1 - uv.y * 2, 0, 1); o.uv = uv; return o;
}
Texture2D image : register(t0); SamplerState linearSampler : register(s0);
float4 ps(V i) : SV_TARGET { return float4(image.Sample(linearSampler, i.uv).rgb, 1); }
"#;
/// D3D11 flip-model swap chain: GPU copy -> shader -> letterboxed preview, no CPU pixels.
pub struct Preview {
    graphics: GraphicsDevice,
    swap: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    size: Dimensions,
    frame_size: Dimensions,
    view: Option<ID3D11ShaderResourceView>,
    texture: Option<ID3D11Texture2D>,
}
impl Preview {
    pub fn new(graphics: GraphicsDevice, hwnd: HWND, size: Dimensions) -> Result<Self> {
        // SAFETY: valid live child HWND and a D3D11 device on this thread; COM owns all objects.
        unsafe {
            let dxgi: IDXGIDevice = graphics.device.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: size.width.max(1),
                Height: size.height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                Scaling: DXGI_SCALING_STRETCH,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&graphics.device, hwnd, &desc, None, None)?;
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
            let mut vs = None;
            let mut ps = None;
            D3DCompile(
                SHADER.as_ptr().cast(),
                SHADER.len(),
                None,
                None,
                None,
                s!("vs"),
                s!("vs_4_0"),
                0,
                0,
                &mut vs,
                None,
            )?;
            D3DCompile(
                SHADER.as_ptr().cast(),
                SHADER.len(),
                None,
                None,
                None,
                s!("ps"),
                s!("ps_4_0"),
                0,
                0,
                &mut ps,
                None,
            )?;
            let vs = vs.unwrap();
            let ps = ps.unwrap();
            let mut vertex = None;
            let mut pixel = None;
            let mut sampler = None;
            graphics.device.CreateVertexShader(
                std::slice::from_raw_parts(vs.GetBufferPointer().cast(), vs.GetBufferSize()),
                None,
                Some(&mut vertex),
            )?;
            graphics.device.CreatePixelShader(
                std::slice::from_raw_parts(ps.GetBufferPointer().cast(), ps.GetBufferSize()),
                None,
                Some(&mut pixel),
            )?;
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                ..Default::default()
            };
            graphics
                .device
                .CreateSamplerState(&desc, Some(&mut sampler))?;
            let mut preview = Self {
                graphics,
                swap,
                target: None,
                vertex: vertex.unwrap(),
                pixel: pixel.unwrap(),
                sampler: sampler.unwrap(),
                size,
                frame_size: Dimensions::default(),
                view: None,
                texture: None,
            };
            preview.create_target()?;
            preview.draw()?;
            Ok(preview)
        }
    }
    fn create_target(&mut self) -> Result<()> {
        // SAFETY: back buffer and render target belong to this swap chain/device.
        unsafe {
            let buffer: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            self.graphics
                .device
                .CreateRenderTargetView(&buffer, None, Some(&mut self.target))
        }
    }
    pub fn resize(&mut self, size: Dimensions) -> Result<()> {
        if size.is_empty() || size == self.size {
            return Ok(());
        }
        // SAFETY: remove all back-buffer references before ResizeBuffers.
        unsafe {
            self.graphics.context.OMSetRenderTargets(None, None);
            self.target = None;
            self.swap.ResizeBuffers(
                0,
                size.width,
                size.height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.size = size;
        self.create_target()?;
        self.draw()
    }
    pub fn present(&mut self, frame: &GpuFrame) -> Result<()> {
        if self.texture.as_ref() != Some(&frame.texture) {
            self.view = None;
            // SAFETY: capture frame is an owned shader-readable D3D11 texture.
            unsafe {
                self.graphics.device.CreateShaderResourceView(
                    &frame.texture,
                    None,
                    Some(&mut self.view),
                )?;
            }
            self.texture = Some(frame.texture.clone());
        }
        self.frame_size = frame.metadata.size;
        self.draw()
    }
    pub fn clear(&mut self) -> Result<()> {
        self.view = None;
        self.texture = None;
        self.frame_size = Dimensions::default();
        self.draw()
    }
    pub fn draw(&mut self) -> Result<()> {
        let Some(target) = &self.target else {
            return Ok(());
        };
        // SAFETY: all pipeline objects belong to the same device; viewport is within target bounds.
        unsafe {
            let context = &self.graphics.context;
            context.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            context.ClearRenderTargetView(target, &[0.035, 0.045, 0.065, 1.0]);
            if self.view.is_some() && !self.frame_size.is_empty() {
                let v = letterbox(self.frame_size, self.size);
                context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: v.x as f32,
                    TopLeftY: v.y as f32,
                    Width: v.size.width as f32,
                    Height: v.size.height as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
                context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                context.VSSetShader(&self.vertex, None);
                context.PSSetShader(&self.pixel, None);
                context.PSSetShaderResources(0, Some(std::slice::from_ref(&self.view)));
                context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                context.Draw(3, 0);
                context.PSSetShaderResources(0, Some(&[None]));
            }
            self.swap.Present(0, DXGI_PRESENT(0)).ok()
        }
    }
}

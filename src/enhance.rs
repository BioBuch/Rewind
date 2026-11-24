use std::mem::size_of;

use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11ComputeShader, ID3D11Device, ID3D11DeviceContext,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11UnorderedAccessView, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_UNORDERED_ACCESS, D3D11_BUFFER_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_IMMUTABLE,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

const CAS_HLSL: &str = r#"
Texture2D<float4> SrcTex : register(t0);
RWTexture2D<float4> DstTex : register(u0);

cbuffer Params : register(b0)
{
    float Strength;
    uint Width;
    uint Height;
    float _Pad;
};

float luma(float3 c)
{
    return dot(c, float3(0.299f, 0.587f, 0.114f));
}

[numthreads(8, 8, 1)]
void main(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= Width || id.y >= Height)
        return;

    int2 p = int2(id.xy);
    int2 maxP = int2(int(Width) - 1, int(Height) - 1);

    float4 c4 = SrcTex.Load(int3(p, 0));
    float3 c = c4.rgb;
    float3 n = SrcTex.Load(int3(clamp(p + int2(0, -1), int2(0, 0), maxP), 0)).rgb;
    float3 s = SrcTex.Load(int3(clamp(p + int2(0,  1), int2(0, 0), maxP), 0)).rgb;
    float3 w = SrcTex.Load(int3(clamp(p + int2(-1, 0), int2(0, 0), maxP), 0)).rgb;
    float3 e = SrcTex.Load(int3(clamp(p + int2( 1, 0), int2(0, 0), maxP), 0)).rgb;

    float lc = luma(c);
    float ln = luma(n);
    float ls = luma(s);
    float lw = luma(w);
    float le = luma(e);

    float minL = min(lc, min(min(ln, ls), min(lw, le)));
    float maxL = max(lc, max(max(ln, ls), max(lw, le)));

    float dMin = minL;
    float dMax = 1.0f - maxL;
    float amp = saturate(min(dMin, dMax) / max(maxL, 1e-5f));
    amp = sqrt(amp);

    float peak = -1.0f / lerp(8.0f, 5.0f, saturate(Strength));
    float weight = amp * peak;
    float invSum = 1.0f / (1.0f + 4.0f * weight);

    float3 outRgb = saturate((c + (n + s + w + e) * weight) * invSum);
    DstTex[id.xy] = float4(outRgb, c4.a);
}
"#;

#[repr(C)]
struct CasParams {
    strength: f32,
    width: u32,
    height: u32,
    _pad: f32,
}

pub struct GpuSharpener {
    width: u32,
    height: u32,
    cs: ID3D11ComputeShader,
    in_tex: ID3D11Texture2D,
    in_srv: ID3D11ShaderResourceView,
    out_tex: ID3D11Texture2D,
    out_uav: ID3D11UnorderedAccessView,
    cb: ID3D11Buffer,
}

impl GpuSharpener {
    pub fn new(device: &ID3D11Device, width: u32, height: u32, strength: f32) -> Option<Self> {
        if width == 0 || height == 0 || strength <= 0.0 {
            return None;
        }
        unsafe {
            let mut blob: Option<ID3DBlob> = None;
            let mut err_blob: Option<ID3DBlob> = None;
            let entry = b"main\0";
            let target = b"cs_5_0\0";
            if D3DCompile(
                CAS_HLSL.as_ptr() as *const _,
                CAS_HLSL.len(),
                PCSTR::null(),
                None,
                None,
                PCSTR(entry.as_ptr()),
                PCSTR(target.as_ptr()),
                1 << 15,
                0,
                &mut blob,
                Some(&mut err_blob),
            )
            .is_err()
            {
                return None;
            }
            let blob = blob?;
            let bytecode = std::slice::from_raw_parts(
                blob.GetBufferPointer() as *const u8,
                blob.GetBufferSize(),
            );
            let mut cs = None;
            device.CreateComputeShader(bytecode, None, Some(&mut cs)).ok()?;
            let cs = cs?;

            let in_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut in_tex = None;
            device.CreateTexture2D(&in_desc, None, Some(&mut in_tex)).ok()?;
            let in_tex = in_tex?;
            let mut in_srv = None;
            device
                .CreateShaderResourceView(&in_tex, None, Some(&mut in_srv))
                .ok()?;
            let in_srv = in_srv?;

            let out_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_UNORDERED_ACCESS.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut out_tex = None;
            device.CreateTexture2D(&out_desc, None, Some(&mut out_tex)).ok()?;
            let out_tex = out_tex?;
            let mut out_uav = None;
            device
                .CreateUnorderedAccessView(&out_tex, None, Some(&mut out_uav))
                .ok()?;
            let out_uav = out_uav?;

            let params = CasParams {
                strength,
                width,
                height,
                _pad: 0.0,
            };
            let cb_desc = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<CasParams>() as u32,
                Usage: D3D11_USAGE_IMMUTABLE,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let init = D3D11_SUBRESOURCE_DATA {
                pSysMem: (&params as *const CasParams) as *const _,
                SysMemPitch: 0,
                SysMemSlicePitch: 0,
            };
            let mut cb = None;
            device
                .CreateBuffer(&cb_desc, Some(&init), Some(&mut cb))
                .ok()?;
            let cb = cb?;

            Some(Self {
                width,
                height,
                cs,
                in_tex,
                in_srv,
                out_tex,
                out_uav,
                cb,
            })
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn apply(
        &self,
        context: &ID3D11DeviceContext,
        src: &ID3D11Texture2D,
    ) -> &ID3D11Texture2D {
        unsafe {
            context.CopyResource(&self.in_tex, src);
            context.CSSetShader(&self.cs, None);
            let srvs = [Some(self.in_srv.clone())];
            let uavs = [Some(self.out_uav.clone())];
            let cbs = [Some(self.cb.clone())];
            context.CSSetShaderResources(0, Some(&srvs));
            context.CSSetUnorderedAccessViews(0, 1, Some(uavs.as_ptr()), None);
            context.CSSetConstantBuffers(0, Some(&cbs));
            context.Dispatch(self.width.div_ceil(8), self.height.div_ceil(8), 1);

            let null_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];
            let null_uav: [Option<ID3D11UnorderedAccessView>; 1] = [None];
            context.CSSetShaderResources(0, Some(&null_srv));
            context.CSSetUnorderedAccessViews(0, 1, Some(null_uav.as_ptr()), None);
            &self.out_tex
        }
    }
}

pub fn cas_sharpen_pixel(c: f32, n: f32, s: f32, w: f32, e: f32, strength: f32) -> f32 {
    if strength <= 0.0 {
        return c.clamp(0.0, 1.0);
    }
    let min_l = c.min(n.min(s).min(w.min(e)));
    let max_l = c.max(n.max(s).max(w.max(e)));
    let amp = (min_l.min(1.0 - max_l) / max_l.max(1e-5))
        .clamp(0.0, 1.0)
        .sqrt();
    let peak = -1.0 / (8.0 + (5.0 - 8.0) * strength.clamp(0.0, 1.0));
    let weight = amp * peak;
    let inv = 1.0 / (1.0 + 4.0 * weight);
    ((c + (n + s + w + e) * weight) * inv).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, SharpenLevel};

    #[test]
    fn cas_preserves_flat_and_sharpens_edge() {
        assert_eq!(cas_sharpen_pixel(0.5, 0.5, 0.5, 0.5, 0.5, 0.30), 0.5);
        let sharpened = cas_sharpen_pixel(0.6, 0.2, 0.6, 0.6, 0.2, 0.55);
        assert!(sharpened > 0.6 && sharpened <= 1.0);
        let cfg: AppConfig = serde_json::from_str("{}").unwrap_or_default();
        assert_eq!(cfg.capture_sharpen, SharpenLevel::Off);
        assert_eq!(SharpenLevel::Light.strength(), 0.30);
    }
}

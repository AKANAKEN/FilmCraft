//! Exact SDR opaque RGBA8 packing from the linear half-float export accumulator.
//!
//! The color lookup values and index calculation match `linear_to_srgb_u8` in
//! `filmcraft-color`; this path deliberately does not use an sRGB texture conversion or `pow`.

const LUT_LEN: u32 = 4096;

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> lut: array<u32>;
@group(0) @binding(2) var<storage, read_write> output: array<u32>;

fn encode(v: f32) -> u32 {
    let index = u32(floor(clamp(v, 0.0, 1.0) * 4095.0 + 0.5));
    return lut[index];
}

@compute @workgroup_size(16, 16, 1)
fn pack(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(source, 0);
    if id.x >= dims.x || id.y >= dims.y { return; }
    let p = textureLoad(source, vec2<i32>(id.xy), 0);
    let index = id.y * dims.x + id.x;
    // WebGPU buffer data is little-endian: this stores R, G, B, 255 in byte order.
    output[index] = encode(p.r) | (encode(p.g) << 8u) | (encode(p.b) << 16u) | 0xff000000u;
}
"#;

pub(crate) struct PackedExport {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    lut: wgpu::Buffer,
    output: Option<(wgpu::Buffer, u64)>,
    readback: Option<(wgpu::Buffer, u64)>,
}

impl PackedExport {
    pub(crate) fn supported(limits: &wgpu::Limits) -> bool {
        limits.max_compute_workgroup_size_x >= 16
            && limits.max_compute_workgroup_size_y >= 16
            && limits.max_compute_invocations_per_workgroup >= 256
            && limits.max_storage_buffers_per_shader_stage >= 2
            && limits.max_storage_buffer_binding_size >= u64::from(LUT_LEN) * 4
    }

    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("filmcraft-packed-export"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filmcraft-packed-export"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(1, true),
                storage_entry(2, false),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filmcraft-packed-export"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("filmcraft-packed-export"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("pack"),
            compilation_options: Default::default(),
            cache: None,
        });
        let table = filmcraft_color::linear_to_srgb_u8_table();
        let mut lut_bytes = Vec::with_capacity(table.len() * 4);
        for value in table {
            lut_bytes.extend_from_slice(&u32::from(*value).to_le_bytes());
        }
        let lut = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filmcraft-srgb-u8-lut"),
            size: u64::from(LUT_LEN) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&lut, 0, &lut_bytes);
        Self { pipeline, bind_group_layout, lut, output: None, readback: None }
    }

    /// Convert a linear premultiplied RGBA16Float accumulator to opaque sRGB RGBA8.
    pub(crate) fn pack(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::TextureView, w: u32, h: u32) -> Option<Vec<u8>> {
        if w == 0 || h == 0 {
            return None;
        }
        let pixels = u64::from(w).checked_mul(u64::from(h))?;
        let bytes = pixels.checked_mul(4)?;
        let limits = device.limits();
        if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
            return None;
        }
        let groups_x = w.div_ceil(16);
        let groups_y = h.div_ceil(16);
        if groups_x > limits.max_compute_workgroups_per_dimension || groups_y > limits.max_compute_workgroups_per_dimension {
            return None;
        }
        ensure_buffer(device, &mut self.output, bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, "packed-export-output");
        ensure_buffer(device, &mut self.readback, bytes, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, "packed-export-readback");
        let (output, _) = self.output.as_ref()?;
        let (readback, _) = self.readback.as_ref()?;
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filmcraft-packed-export"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 1, resource: self.lut.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() },
            ],
        });
        let submitted = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("filmcraft-packed-export") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("filmcraft-packed-export"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }
        encoder.copy_buffer_to_buffer(output, 0, readback, 0, bytes);
        queue.submit([encoder.finish()]);
        super::export_renderer::timing::add_submit(submitted.elapsed());

        let waiting = std::time::Instant::now();
        let slice = readback.slice(0..bytes);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        if super::wait_mapped(device).is_none() || !matches!(rx.recv().ok(), Some(Ok(()))) {
            readback.unmap();
            return None;
        }
        super::export_renderer::timing::add_map_wait(waiting.elapsed());
        let mapped = match slice.get_mapped_range() {
            Ok(mapped) => mapped,
            Err(_) => {
                readback.unmap();
                return None;
            }
        };
        let mut result = Vec::new();
        if usize::try_from(bytes).ok().is_none_or(|n| result.try_reserve_exact(n).is_err()) {
            drop(mapped);
            readback.unmap();
            return None;
        }
        result.extend_from_slice(&mapped);
        drop(mapped);
        readback.unmap();
        Some(result)
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn ensure_buffer(device: &wgpu::Device, slot: &mut Option<(wgpu::Buffer, u64)>, size: u64, usage: wgpu::BufferUsages, label: &'static str) {
    if slot.as_ref().is_none_or(|(_, old)| *old < size) {
        *slot = Some((device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false }), size));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        if wgpu::Instance::enabled_backend_features().is_empty() {
            return None;
        }
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn check(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, pixels: &[[u16; 4]], packer: &mut PackedExport) {
        let mut bytes = Vec::with_capacity(pixels.len() * 8);
        for p in pixels {
            for channel in p {
                bytes.extend_from_slice(&channel.to_le_bytes());
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("packed-export-test-source"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&Default::default());
        let got = packer.pack(device, queue, &view, w, h).expect("packed readback");
        let mut expected = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            for channel in &p[..3] {
                expected.push(filmcraft_color::linear_to_srgb_u8(super::super::f16_to_f32(*channel)));
            }
            expected.push(255);
        }
        assert_eq!(got, expected, "{w}x{h}");
    }

    #[test]
    fn packed_rgba8_matches_cpu_for_odd_edges_alpha_and_clamped_values() {
        let Some((device, queue)) = gpu() else {
            eprintln!("SKIPPED (packed export): no GPU adapter");
            return;
        };
        let w = 13;
        let h = 7;
        let values = [0x0000, 0x8000, 0x3c00, 0x3800, 0x7bff, 0xfbff, 0x0400, 0x3555, 0x7c00, 0xfc00];
        let pixels: Vec<[u16; 4]> = (0..w * h)
            .map(|i| {
                [
                    values[(i * 3 % values.len() as u32) as usize],
                    values[(i * 5 % values.len() as u32) as usize],
                    values[(i * 7 % values.len() as u32) as usize],
                    (i * 311) as u16,
                ]
            })
            .collect();
        check(&device, &queue, w, h, &pixels, &mut PackedExport::new(&device, &queue));
    }

    #[test]
    fn packed_rgba8_matches_cpu_for_every_finite_half_value() {
        let Some((device, queue)) = gpu() else {
            eprintln!("SKIPPED (packed export): no GPU adapter");
            return;
        };
        // Include every finite binary16 bit pattern, with distinct channel permutations and alpha.
        let pixels: Vec<[u16; 4]> = (0..=u16::MAX).filter(|bits| bits & 0x7c00 != 0x7c00).map(|bits| [bits, bits ^ 0x8000, bits, bits ^ 0x8000]).collect();
        let (w, h) = (256, 256);
        let mut padded = pixels;
        padded.resize((w * h) as usize, [0; 4]);
        check(&device, &queue, w, h, &padded, &mut PackedExport::new(&device, &queue));
    }

    #[test]
    fn packed_buffers_reuse_across_sizes_and_decline_invalid_dimensions() {
        let Some((device, queue)) = gpu() else {
            eprintln!("SKIPPED (packed buffers): no GPU adapter");
            return;
        };
        let mut packer = PackedExport::new(&device, &queue);
        for (w, h) in [(16, 16), (13, 7), (32, 19), (1, 1)] {
            let pixels = vec![[0x3555, 0x3c00, 0x3800, 0]; (w * h) as usize];
            check(&device, &queue, w, h, &pixels, &mut packer);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        assert!(packer.pack(&device, &queue, &view, 0, 1).is_none());
        assert!(packer.pack(&device, &queue, &view, u32::MAX, u32::MAX).is_none());
    }

    #[test]
    fn devices_without_sufficient_compute_limits_keep_the_float_route() {
        let mut limits = wgpu::Limits::default();
        assert!(PackedExport::supported(&limits));
        limits.max_compute_invocations_per_workgroup = 128;
        assert!(!PackedExport::supported(&limits));
        limits.max_compute_invocations_per_workgroup = 256;
        limits.max_storage_buffers_per_shader_stage = 1;
        assert!(!PackedExport::supported(&limits));
    }
}

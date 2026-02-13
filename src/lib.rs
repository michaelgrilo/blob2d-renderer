#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use gloo::utils::window;
    use web_sys::HtmlCanvasElement;

    pub struct Renderer {
        canvas: HtmlCanvasElement,
        backend: wgpu::Backend,
        surface: wgpu::Surface<'static>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: wgpu::SurfaceConfiguration,
        render_pipeline: wgpu::RenderPipeline,
        texture_bind_group_layout: wgpu::BindGroupLayout,
        sampler: wgpu::Sampler,
        texture: wgpu::Texture,
        texture_bind_group: wgpu::BindGroup,
    }

    impl Renderer {
        pub async fn new(canvas: HtmlCanvasElement) -> Result<Self, String> {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..Default::default()
            });

            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .map_err(|err| format!("failed to create canvas surface: {err}"))?;

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .ok_or_else(|| "failed to request WebGL2 adapter".to_string())?;

            let backend = adapter.get_info().backend;

            let required_limits =
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
            let (device, queue) = adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: Some("blob2d-renderer-device"),
                        required_features: wgpu::Features::empty(),
                        required_limits,
                    },
                    None,
                )
                .await
                .map_err(|err| format!("failed to request WebGL2 device: {err}"))?;

            let surface_caps = surface.get_capabilities(&adapter);
            let format = surface_caps
                .formats
                .iter()
                .copied()
                .find(wgpu::TextureFormat::is_srgb)
                .unwrap_or(surface_caps.formats[0]);
            let present_mode = surface_caps
                .present_modes
                .iter()
                .copied()
                .find(|mode| *mode == wgpu::PresentMode::Fifo)
                .unwrap_or(surface_caps.present_modes[0]);
            let alpha_mode = surface_caps.alpha_modes[0];

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: 1,
                height: 1,
                present_mode,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };

            surface.configure(&device, &config);

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("blob2d-renderer-shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
            });

            let texture_bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("blob2d-renderer-texture-layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("blob2d-renderer-pipeline-layout"),
                bind_group_layouts: &[&texture_bind_group_layout],
                push_constant_ranges: &[],
            });

            let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("blob2d-renderer-render-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("blob2d-renderer-sampler"),
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::FilterMode::Nearest,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            });

            let (texture, texture_bind_group) = create_texture_and_bind_group(
                &device,
                &queue,
                &texture_bind_group_layout,
                &sampler,
                &[244, 231, 208, 255],
                1,
                1,
            );

            Ok(Self {
                canvas,
                backend,
                surface,
                device,
                queue,
                config,
                render_pipeline,
                texture_bind_group_layout,
                sampler,
                texture,
                texture_bind_group,
            })
        }

        pub fn backend_label(&self) -> &'static str {
            match self.backend {
                wgpu::Backend::Empty => "Empty",
                wgpu::Backend::Vulkan => "Vulkan",
                wgpu::Backend::Metal => "Metal",
                wgpu::Backend::Dx12 => "DirectX12",
                wgpu::Backend::Gl => "WebGL2",
                wgpu::Backend::BrowserWebGpu => "BrowserWebGPU",
            }
        }

        pub fn canvas_size(&self) -> (u32, u32) {
            (self.config.width, self.config.height)
        }

        pub fn resize(&mut self) {
            let device_pixel_ratio = window().device_pixel_ratio().max(1.0);
            let width =
                ((self.canvas.client_width().max(1) as f64) * device_pixel_ratio).round() as u32;
            let height =
                ((self.canvas.client_height().max(1) as f64) * device_pixel_ratio).round() as u32;

            if width == self.config.width && height == self.config.height {
                return;
            }

            self.canvas.set_width(width);
            self.canvas.set_height(height);
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }

        pub fn upload_image(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
            if width == 0 || height == 0 {
                return Err("pasted image has invalid dimensions".to_string());
            }

            let (texture, texture_bind_group) = create_texture_and_bind_group(
                &self.device,
                &self.queue,
                &self.texture_bind_group_layout,
                &self.sampler,
                rgba,
                width,
                height,
            );

            self.texture = texture;
            self.texture_bind_group = texture_bind_group;

            Ok(())
        }

        pub fn render(&mut self) -> Result<(), String> {
            let frame = match self.surface.get_current_texture() {
                Ok(frame) => frame,
                Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                    self.surface.configure(&self.device, &self.config);
                    self.surface
                        .get_current_texture()
                        .map_err(|err| format!("failed to recover swap chain frame: {err}"))?
                }
                Err(wgpu::SurfaceError::OutOfMemory) => {
                    return Err("surface ran out of memory".to_string());
                }
                Err(wgpu::SurfaceError::Timeout) => {
                    return Err("surface timed out while waiting for frame".to_string());
                }
            };

            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("blob2d-renderer-render-encoder"),
                });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blob2d-renderer-render-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.95,
                                g: 0.91,
                                b: 0.84,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });

                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_bind_group(0, &self.texture_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            self.queue.submit(Some(encoder.finish()));
            frame.present();

            Ok(())
        }
    }

    fn create_texture_and_bind_group(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::BindGroup) {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blob2d-renderer-image-texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blob2d-renderer-texture-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        (texture, texture_bind_group)
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::Renderer;

#[cfg(not(target_arch = "wasm32"))]
mod native_stub {
    use web_sys::HtmlCanvasElement;

    pub struct Renderer;

    impl Renderer {
        pub async fn new(_canvas: HtmlCanvasElement) -> Result<Self, String> {
            Err("blob2d-renderer is only available on wasm32 targets".to_string())
        }

        pub fn backend_label(&self) -> &'static str {
            "Unavailable"
        }

        pub fn canvas_size(&self) -> (u32, u32) {
            (0, 0)
        }

        pub fn resize(&mut self) {}

        pub fn upload_image(
            &mut self,
            _rgba: &[u8],
            _width: u32,
            _height: u32,
        ) -> Result<(), String> {
            Ok(())
        }

        pub fn render(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native_stub::Renderer;

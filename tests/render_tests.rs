use tacky_borders::animations::Animations;
use tacky_borders::border_drawer::BorderDrawer;
use tacky_borders::colors::ColorBrush;
use tacky_borders::effects::Effects;
use tacky_borders::register_border_window_class;
use tacky_borders::render_backend::{RenderBackend, RenderBackendConfig};
use tacky_borders::window_border::{WindowBorder, WindowState};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;

#[test]
fn test_render_backend_v2_with_extra_bitmaps() -> anyhow::Result<()> {
    let mut border_window = WindowBorder::default();
    register_border_window_class()?;
    let hwnd = border_window.create_window()?;

    let render_backend = RenderBackendConfig::V2.to_render_backend(1920, 1080, hwnd, true)?;
    if let RenderBackend::V2(ref backend) = render_backend {
        assert!(backend.mask_bitmap.is_some());
        assert!(backend.border_bitmap.is_some());
        assert!(
            render_backend.get_pixel_size()?
                == D2D_SIZE_U {
                    width: 1920,
                    height: 1080
                }
        );
    } else {
        panic!("created incorrect render backend");
    }

    Ok(())
}

#[test]
fn test_render_backend_v2_without_extra_bitmaps() -> anyhow::Result<()> {
    let mut border_window = WindowBorder::default();
    register_border_window_class()?;
    let hwnd = border_window.create_window()?;

    let render_backend = RenderBackendConfig::V2.to_render_backend(1920, 1080, hwnd, false)?;
    if let RenderBackend::V2(ref backend) = render_backend {
        assert!(backend.mask_bitmap.is_none());
        assert!(backend.border_bitmap.is_none());
        assert!(
            render_backend.get_pixel_size()?
                == D2D_SIZE_U {
                    width: 1920,
                    height: 1080
                }
        );
    } else {
        panic!("created incorrect render backend");
    }

    Ok(())
}

#[test]
fn test_border_drawer_update() -> anyhow::Result<()> {
    let mut border_window = WindowBorder::default();
    let mut border_drawer = BorderDrawer::default();

    register_border_window_class()?;
    let hwnd = border_window.create_window()?;

    border_drawer.configure_border(
        4,
        -1,
        8.0,
        ColorBrush::default(),
        ColorBrush::default(),
        Animations::default(),
        Effects::default(),
    );
    border_drawer.init_renderer(1920, 1080, hwnd, &RECT::default(), RenderBackendConfig::V2)?;

    assert!(
        border_drawer
            .render(&RECT::default(), 0, WindowState::default())
            .is_ok()
    );
    assert!(border_drawer.update_renderer(1280, 720).is_ok());
    assert!(
        border_drawer
            .render(&RECT::default(), 0, WindowState::default())
            .is_ok()
    );
    assert!(
        border_drawer.render_backend.get_pixel_size()?
            == D2D_SIZE_U {
                width: 1280,
                height: 720
            }
    );

    Ok(())
}

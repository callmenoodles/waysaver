use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration};
use gdk4_wayland::prelude::ApplicationExt;
use gtk4::{Application, ApplicationWindow, EventControllerMotion, EventControllerKey, CssProvider};
use gtk4::prelude::*;
use gtk4_layer_shell::*;
use webkit6::prelude::WebViewExt;
use webkit6::WebView;
use anyhow::{Result};
use gdk4_wayland::gdk::{Cursor, Display, Monitor};
use gdk4_wayland::glib::{ExitCode};
use gtk4::gio::ListModel;
use clap::Parser;
use std::path::{Path, PathBuf};

const MOUSE_THRESHOLD: f64 = 10.0;

#[derive(Parser)]
#[command(author, version, about = "Screensaver for wlroots-based Wayland compositors", long_about = None)]
struct Args {
    #[arg(help = "URL or local HTML file path to display")]
    html: String,
}

#[derive(Debug)]
struct InputState {
    should_close: bool,
    last_mouse_pos: Option<(f64, f64)>,
    total_movement: f64,
}

impl InputState {
    fn new() -> Self {
        Self {
            should_close: false,
            last_mouse_pos: None,
            total_movement: 0.0,
        }
    }

    fn handle_mouse_movement(&mut self, x: f64, y: f64) -> bool {
        if let Some((last_x, last_y)) = self.last_mouse_pos {
            let dx: f64 = x - last_x;
            let dy: f64 = y - last_y;
            let distance: f64 = (dx * dx + dy * dy).sqrt();

            self.total_movement += distance;

            if self.total_movement > MOUSE_THRESHOLD {
                self.should_close = true;
                return true;
            }
        }

        self.last_mouse_pos = Some((x, y));
        false
    }

    fn handle_key_input(&mut self) -> bool {
        self.should_close = true;
        true
    }

    fn should_close(&self) -> bool {
        self.should_close
    }
}

fn resolve_content_url(content: &str) -> Result<String> {
    if content.starts_with("http://") || content.starts_with("https://") {
        return Ok(content.to_string());
    }
    
    if content.starts_with("file://") {
        return Ok(content.to_string());
    }
    
    let path: &Path = Path::new(content);
    
    if !path.exists() {
        anyhow::bail!("File does not exist: {}", content);
    }
    
    let absolute_path: PathBuf = std::fs::canonicalize(path)
        .map_err(|e| anyhow::anyhow!("Failed to resolve file path '{}': {}", content, e))?;
    
    Ok(format!("file://{}", absolute_path.to_string_lossy()))
}

fn main() -> Result<ExitCode> {
    let args: Args = Args::parse();
    let url: String = resolve_content_url(&args.html)?;
    
    gtk4::init().expect("Failed to initialize GTK");

    let display: Display = Display::default().expect("Could not connect to a display");
    let monitors: ListModel = display.monitors();

    let application: Application = Application::builder()
        .build();

    // Arc allows multiple references to the same data
    // Mutex allows thread-safe access to the data
    let input_state: Arc<Mutex<InputState>> = Arc::new(Mutex::new(InputState::new()));
    let windows: Arc<Mutex<Vec<ApplicationWindow>>> = Arc::new(Mutex::new(Vec::new()));
    
    let input_state_activate: Arc<Mutex<InputState>> = input_state.clone();
    let windows_activate: Arc<Mutex<Vec<ApplicationWindow>>> = windows.clone();
    
    let blank_cursor: Option<Cursor> = Cursor::from_name("none", None);
    
    application.connect_activate(move |app: &Application| {
        let mut window_list: MutexGuard<Vec<ApplicationWindow>> = windows_activate.lock().unwrap();

        for i in 0..monitors.n_items() {
            if let Some(obj) = monitors.item(i) {
                match obj.downcast::<Monitor>() {
                    Ok(monitor) => {
                        let window: ApplicationWindow = ApplicationWindow::builder()
                            .application(app)
                            .build();
                            
                        window.init_layer_shell();
                        window.set_cursor(blank_cursor.as_ref());
                        window.set_can_target(true);
                        window.set_can_focus(true);
                        window.set_focus_visible(false);
                        window.set_hide_on_close(false);
                        window.set_destroy_with_parent(true);
                        window.set_monitor(Some(&monitor));
                        window.set_namespace(Some("waysaver"));
                        window.set_layer(Layer::Overlay);
                        window.set_exclusive_zone(-1);
                        window.set_decorated(false);
                        
                        window.set_anchor(Edge::Top, true);
                        window.set_anchor(Edge::Bottom, true);
                        window.set_anchor(Edge::Left, true);
                        window.set_anchor(Edge::Right, true);
                        
                        window.set_keyboard_mode(KeyboardMode::Exclusive);
                        
                        let motion_controller: EventControllerMotion = EventControllerMotion::new();
                        let input_state_motion: Arc<Mutex<InputState>> = input_state_activate.clone();

                        motion_controller.connect_motion(move |_, x: f64, y: f64| {
                            let mut state: MutexGuard<InputState> = input_state_motion.lock().unwrap();
                            state.handle_mouse_movement(x, y);
                        });

                        window.add_controller(motion_controller);
                        
                        let key_controller: EventControllerKey = EventControllerKey::new();
                        let input_state_key_press: Arc<Mutex<InputState>> = input_state_activate.clone();

                        key_controller.connect_key_pressed(move |_, _, _keycode, _state| {
                            let mut state: MutexGuard<InputState> = input_state_key_press.lock().unwrap();
                            state.handle_key_input();
                            gdk4_wayland::glib::Propagation::Stop
                        });
                        
                        let input_state_key_rel: Arc<Mutex<InputState>> = input_state_activate.clone();

                        key_controller.connect_key_released(move |_, _, _keycode, _state| {
                            let mut state: MutexGuard<InputState> = input_state_key_rel.lock().unwrap();
                            state.handle_key_input();
                        });

                        window.add_controller(key_controller);
                        
                        let window_focus: ApplicationWindow = window.clone();
                        window.connect_notify_local(Some("is-active"), move |win: &ApplicationWindow, _| {
                            if !win.is_active() {
                                window_focus.present();
                                window_focus.grab_focus();
                            }
                        });
                        
                        window.set_css_classes(&["screensaver-window"]);
                        
                        let css_provider: CssProvider = gtk4::CssProvider::new();
                        
                        css_provider.load_from_string("
                            .screensaver-window {
                                background-color: black;
                            }
                            ");

                        let display: Display = gdk4_wayland::gdk::Display::default().unwrap();
                        
                        gtk4::style_context_add_provider_for_display(
                            &display,
                            &css_provider,
                            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                        );

                        let webview: WebView = WebView::new();
                        webview.set_background_color(&gdk4_wayland::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0)); // Black background
                        webview.set_can_focus(false); 
                        webview.set_editable(false);
                        webview.set_can_target(false);
                        webview.load_uri(&url);
                        window.set_child(Some(&webview));
                        window.set_opacity(0.0);
                        
                        window.present();
                        window.grab_focus();
                        
                        let window_fade: ApplicationWindow = window.clone();
                        let mut opacity: f64 = 0.0;
                        let fade_duration_ms: i32 = 200;
                        let frame_time_ms: u64 = 16;
                        let opacity_step: f64 = 1.0 / (fade_duration_ms as f64 / frame_time_ms as f64);

                        gdk4_wayland::glib::timeout_add_local(Duration::from_millis(frame_time_ms), move || {
                            opacity += opacity_step;
                            if opacity >= 1.0 {
                                opacity = 1.0;
                                window_fade.set_opacity(opacity);
                                gdk4_wayland::glib::ControlFlow::Break
                            } else {
                                window_fade.set_opacity(opacity);
                                gdk4_wayland::glib::ControlFlow::Continue
                            }
                        });

                        window_list.push(window);
                    }
                    Err(_) => {
                        eprintln!("Failed to downcast monitor object");
                    }
                }
            }
        }
        
        drop(window_list);
        
        let input_state_timer: Arc<Mutex<InputState>> = input_state_activate.clone();
        let windows_timer: Arc<Mutex<Vec<ApplicationWindow>>> = windows_activate.clone();
        let app_timer: Application = app.clone();

        gdk4_wayland::glib::timeout_add_local(Duration::from_millis(50), move || {
            {
                let state: MutexGuard<InputState> = input_state_timer.lock().unwrap();
                if state.should_close() {
                    drop(state);
                    println!("Closing screensaver...");
                    close(&windows_timer, &app_timer);
                    return gdk4_wayland::glib::ControlFlow::Break;
                }
            }

            let window_list: MutexGuard<Vec<ApplicationWindow>> = windows_timer.lock().unwrap();
            for window in window_list.iter() {
                if !window.is_active() {
                    window.present();
                    window.grab_focus();
                }
            }
            drop(window_list);

            gdk4_wayland::glib::ControlFlow::Continue
        });
    });

    Ok(application.run_with_args::<String>(&[]))
}

fn close(windows: &Arc<Mutex<Vec<ApplicationWindow>>>, app: &Application) {
    let windows: MutexGuard<Vec<ApplicationWindow>> = windows.lock().unwrap();
    
    for window in windows.iter() {
        window.close();
    }
    
    drop(windows);
    app.quit();
}

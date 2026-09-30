use std::cell::Cell;
use std::ffi::CString;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread::JoinHandle;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use ringbuf::HeapCons;
use ringbuf::traits::{Consumer, Observer};

use crate::dsp::Frame;
use crate::telemetry::Logger;

const CHANNELS: usize = 2;
const STRIDE: usize = CHANNELS * std::mem::size_of::<f32>();

// The app plays into this virtual speaker, and PipeWire loops it back out as the
// "rust-phone" microphone.
const SINK_NAME: &str = "rust-phone-input";
const CONFIG_FILE: &str = "rust-phone.conf";

// Used both in the config file (PipeWire creates the device at every login, before
// any browser starts) and loaded in-process when the device doesn't exist yet.
const LOOPBACK_ARGS: &str = r#"{
    node.description = "Rust Phone"
    audio.position = [ FL FR ]
    capture.props = {
        node.name = "rust-phone-input"
        node.description = "Rust Phone (app output)"
        media.class = "Audio/Sink"
    }
    playback.props = {
        node.name = "rust-phone"
        node.description = "Rust Phone"
        media.class = "Audio/Source"
    }
}"#;

/// Makes sure the "Rust Phone" microphone exists and streams the pipeline's frames into it.
pub fn spawn(
    consumer: HeapCons<Frame>,
    sample_rate: u32,
    latency_frames: usize,
    logger: Logger,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        if let Err(err) = run(consumer, sample_rate, latency_frames, &logger) {
            logger.error(format!("virtual mic stopped: {err}"));
        }
    })
}

fn run(
    consumer: HeapCons<Frame>,
    sample_rate: u32,
    latency_frames: usize,
    logger: &Logger,
) -> Result<(), pw::Error> {
    match install_config() {
        Ok(Some(path)) => logger.info(format!(
            "Installed {}: \"Rust Phone\" will exist from your next login on.",
            path.display()
        )),
        Ok(None) => {}
        Err(err) => logger.error(format!("could not install the Rust Phone config: {err}")),
    }

    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    // Keeps the in-process device alive (if we had to create one) until this thread ends.
    let _device = if node_exists(&mainloop, &core, SINK_NAME)? {
        None
    } else {
        let module = load_loopback(&context);
        if module.is_none() {
            logger.error("could not load libpipewire-module-loopback");
        }
        if wait_for_node(&mainloop, &core, SINK_NAME)? {
            logger.warn(
                "\"Rust Phone\" created for this session only. Restart your browser once so it lists it.",
            );
        } else {
            logger.error(format!(
                "\"{SINK_NAME}\" did not show up in PipeWire; the stream may not connect"
            ));
        }
        module
    };

    let stream = pw::stream::StreamBox::new(
        &core,
        "rust-phone-feed",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::NODE_NAME => "rust-phone-feed",
            *pw::keys::AUDIO_CHANNELS => "2",
            *pw::keys::NODE_LATENCY => format!("{latency_frames}/{sample_rate}"),
            *pw::keys::TARGET_OBJECT => SINK_NAME,
            // Never fall back to the real speakers if the device goes away.
            *pw::keys::NODE_DONT_RECONNECT => "true",
            "node.dont-fallback" => "true",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(consumer)
        .process(move |stream, consumer| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            // The buffer can hold far more than one block; only fill what PipeWire asked for.
            let requested = buffer.requested() as usize;
            let data = &mut buffer.datas_mut()[0];
            let Some(bytes) = data.data() else {
                return;
            };
            let mut n_frames = bytes.len() / STRIDE;
            if requested > 0 {
                n_frames = n_frames.min(requested);
            }

            // Until an app records from the mic, nothing drains the queue and it
            // fills up. Dropping the backlog keeps the delay at ~one mic block.
            let max_queued = n_frames + latency_frames;
            let excess = consumer.occupied_len().saturating_sub(max_queued);
            consumer.skip(excess);

            for out in bytes[..n_frames * STRIDE].chunks_exact_mut(STRIDE) {
                let [l, r] = consumer.try_pop().unwrap_or([0.0, 0.0]);
                out[..4].copy_from_slice(&l.to_le_bytes());
                out[4..].copy_from_slice(&r.to_le_bytes());
            }

            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = STRIDE as _;
            *chunk.size_mut() = (n_frames * STRIDE) as _;
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(CHANNELS as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);

    let format: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .expect("audio format pod serializes")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&format).expect("valid audio format pod")];

    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    mainloop.run();
    Ok(())
}

/// Writes the PipeWire config that creates the device at login. Returns the path
/// when it was written now, `None` when it was already there.
fn install_config() -> std::io::Result<Option<PathBuf>> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| std::io::Error::other("neither XDG_CONFIG_HOME nor HOME is set"))?;
    let dir = config_home.join("pipewire/pipewire.conf.d");
    let path = dir.join(CONFIG_FILE);
    if path.exists() {
        return Ok(None);
    }

    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        &path,
        format!(
            "# Created by Rust Phone. Delete this file to remove the \"Rust Phone\" microphone.\n\
             context.modules = [\n    {{ name = libpipewire-module-loopback\n      args = {LOOPBACK_ARGS}\n    }}\n]\n"
        ),
    )?;
    Ok(Some(path))
}

/// Loads the loopback inside this process. Returned handle is freed with the context.
fn load_loopback(context: &pw::context::ContextRc) -> Option<*mut pw::sys::pw_impl_module> {
    let name = CString::new("libpipewire-module-loopback").expect("no NUL in module name");
    let args = CString::new(LOOPBACK_ARGS).expect("no NUL in module args");
    let module = unsafe {
        pw::sys::pw_context_load_module(
            context.as_raw_ptr(),
            name.as_ptr(),
            args.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    (!module.is_null()).then_some(module)
}

/// Asks the server for all current objects and reports whether a node with `name` is among them.
fn node_exists(
    mainloop: &pw::main_loop::MainLoopRc,
    core: &pw::core::CoreRc,
    name: &str,
) -> Result<bool, pw::Error> {
    let registry = core.get_registry()?;
    let found = Rc::new(Cell::new(false));
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let found = found.clone();
            let name = name.to_owned();
            move |global| {
                let node_name = global
                    .props
                    .as_ref()
                    .and_then(|props| props.get("node.name"));
                if node_name == Some(name.as_str()) {
                    found.set(true);
                }
            }
        })
        .register();

    // The server answers the sync only after it has sent every existing object.
    let pending = core.sync(0)?;
    let done = Rc::new(Cell::new(false));
    let _core_listener = core
        .add_listener_local()
        .done({
            let done = done.clone();
            let mainloop = mainloop.clone();
            move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq == pending {
                    done.set(true);
                    mainloop.quit();
                }
            }
        })
        .register();
    while !done.get() {
        mainloop.run();
    }
    Ok(found.get())
}

/// A freshly loaded module registers its nodes asynchronously; give it a few roundtrips.
fn wait_for_node(
    mainloop: &pw::main_loop::MainLoopRc,
    core: &pw::core::CoreRc,
    name: &str,
) -> Result<bool, pw::Error> {
    for _ in 0..20 {
        if node_exists(mainloop, core, name)? {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(false)
}

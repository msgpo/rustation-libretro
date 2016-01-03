pub mod libretro;
mod retrogl;
mod retrolog;

use std::path::{Path, PathBuf};
use std::fs::File;
use std::io::Read;

use libc::{c_char, c_uint};

use rustation::{Disc, Region};
use rustation::bios::{Bios, BIOS_SIZE};

extern crate libc;
extern crate gl;
#[macro_use]
extern crate log;
extern crate rustation;

macro_rules! cstring {
    ($x:expr) => {
        concat!($x, '\0') as *const _ as *const c_char
    };
}

/// Static system information sent to the frontend on request
const SYSTEM_INFO: libretro::SystemInfo = libretro::SystemInfo {
    library_name: cstring!("Rustation"),
    library_version: rustation::VERSION_CSTR as *const _ as *const c_char,
    valid_extensions: cstring!("bin"),
    need_fullpath: false,
    block_extract: false,
};

/// Emulator context
struct Context {
    retrogl: retrogl::RetroGl,
}

impl Context {
    fn new(disc: &Path) -> Result<Context, ()> {

        let disc =
            match Disc::from_path(&disc) {
                Ok(d) => d,
                Err(e) => {
                    error!("Couldn't load {}: {}", disc.to_string_lossy(), e);
                    return Err(());
                }
            };

        let region = disc.region();

        info!("Detected disc region: {:?}", region);

        let _bios =
            match find_bios(region) {
                Some(b) => b,
                None => {
                    error!("Couldn't find a BIOS, bailing out");
                    return Err(());
                }
            };

        let retrogl = try!(retrogl::RetroGl::new());

        Ok(Context {
            retrogl: retrogl,
        })
    }
}

impl libretro::Context for Context {

    fn render_frame(&mut self) {
        match self.retrogl.state() {
            Some(s) => {
                if let Err(e) = s.render_frame() {
                    error!("Couldn't render frame: {:?}", e);
                }
            }
            None => {
                error!("Frame requested while we have no RetroGL state!");
                return;
            }
        }

        libretro::gl_frame_done(self.retrogl.xres(), self.retrogl.yres())
    }

    fn get_system_av_info(&self) -> libretro::SystemAvInfo {
        libretro::SystemAvInfo {
            geometry: libretro::GameGeometry {
                base_width: self.retrogl.xres() as c_uint,
                base_height: self.retrogl.yres() as c_uint,
                max_width: 640,
                max_height: 576,
                aspect_ratio: -1.0,
            },
            timing: libretro::SystemTiming {
                fps: 60.,
                sample_rate: 44_100.
            }
        }
    }

    fn gl_context_reset(&mut self) {
        self.retrogl.context_reset();
    }

    fn gl_context_destroy(&mut self) {
        self.retrogl.context_destroy();
    }
}

/// Init function, called only once when our core gets loaded
fn init() {
    retrolog::init();
}

/// Called when a game is loaded and a new context must be built
fn load_game(disc: PathBuf) -> Option<Box<libretro::Context>> {
    info!("Loading {:?}", disc);

    Context::new(&disc).ok()
        .map(|c| Box::new(c) as Box<libretro::Context>)
}

/// Attempt to find a BIOS for `region` in the system directory
fn find_bios(region: Region) -> Option<Bios> {
    let system_directory =
        match libretro::get_system_directory() {
            Some(dir) => dir,
            // libretro.h says that when the system directory is not
            // provided "it's up to the implementation to find a
            // suitable directory" but I'm not sure what to put
            // here. Maybe "."? I'd rather give an explicit error
            // message instead.
            None => {
                error!("The frontend didn't give us a system directory, \
                        no BIOS can be loaded");
                return None;
            }
        };

    info!("Looking for a BIOS for region {:?} in {:?}",
          region,
          system_directory);

    let dir =
        match ::std::fs::read_dir(&system_directory) {
            Ok(d) => d,
            Err(e) => {
                error!("Can't read directory {:?}: {}",
                       system_directory, e);
                return None;
            }
        };

    for entry in dir {
        match entry {
            Ok(entry) => {
                let path = entry.path();

                match entry.metadata() {
                    Ok(md) => {
                        if !md.is_file() {
                            debug!("Ignoring {:?}: not a file", path);
                        } else if md.len() != BIOS_SIZE as u64 {
                            debug!("Ignoring {:?}: bad size", path);
                        } else {
                            let bios = try_bios(region, &path);

                            if bios.is_some() {
                                // Found a valid BIOS!
                                return bios;
                            }
                        }
                    }
                    Err(e) =>
                        warn!("Ignoring {:?}: can't get file metadata: {}",
                              path, e)
                }
            }
            Err(e) => warn!("Error while reading directory: {}", e),
        }
    }

    None
}

/// Attempt to read and load the BIOS at `path`
fn try_bios(region: Region, path: &Path) -> Option<Bios> {

    let mut file =
        match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("Can't open {:?}: {}", path, e);
                return None;
            }
        };

    // Load the BIOS
    let mut data = Box::new([0; BIOS_SIZE]);
    let mut nread = 0;

    while nread < BIOS_SIZE {
        nread +=
            match file.read(&mut data[nread..]) {
                Ok(0) => {
                    warn!("Short read while loading {:?}", path);
                    return None;
                }
                Ok(n) => n,
                Err(e) => {
                    warn!("Error while reading {:?}: {}", path, e);
                    return None;
                }
            };
    }

    match Bios::new(data) {
        Some(bios) => {
            let md = bios.metadata();

            if md.known_bad {
                warn!("Ignoring {:?}: known bad dump", path);
                None
            } else if md.region != region {
                info!("Ignoring {:?}: bad region ({:?})", path, md.region);
                None
            } else {
                info!("Using BIOS {:?} ({:?}, version {}.{})",
                      path,
                      md.region,
                      md.version_major,
                      md.version_minor);
                Some(bios)
            }
        }
        None => {
            debug!("Ignoring {:?}: not a known PlayStation BIOS", path);
            None
        }
    }
}

//  This file is part of Sulis, a turn based RPG written in Rust.
//  Copyright 2018 Jared Stephen
//
//  Sulis is free software: you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  Sulis is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with Sulis.  If not, see <http://www.gnu.org/licenses/>

mod point;
pub use self::point::{Offset, Point, Rect, Scale};

pub mod size;
pub use self::size::Size;

use std::cell::RefCell;
use std::cmp::Ordering;
use std::f32;
use std::fmt;
use std::fs;
use std::io::{Error, ErrorKind};
use std::ops::*;
use std::panic;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::{thread, time};

use backtrace::Backtrace;
use log::LevelFilter;
use flexi_logger::{opt_format, Duplicate, Logger, LogSpecBuilder};
use rand::prelude::*;
use rand::{self, distributions::{WeightedIndex,uniform::SampleUniform}, seq::SliceRandom, Rng};
use rand_pcg::Pcg64Mcg;

use crate::config::{self, Config};
use crate::io::{Audio, MainLoopUpdater, System};
use crate::resource::write_to_file;
use crate::ui::Widget;

const MAX_ULPS: i32 = 100;
const MAX_DIFF: f32 = 2.0 * std::f32::EPSILON;

pub fn approx_eq(a: f32, b: f32) -> bool {
    if (a - b).abs() <= MAX_DIFF {
        return true;
    }

    if a.signum() != b.signum() {
        return false;
    }

    let a_int = a.to_bits() as i32;
    let b_int = b.to_bits() as i32;

    i32::abs(a_int - b_int) <= MAX_ULPS
}

#[derive(Clone)]
pub struct ReproducibleRandom {
    seed: u128,
    gen: Pcg64Mcg,
}

impl ReproducibleRandom {
    pub fn new(seed: Option<u128>) -> ReproducibleRandom {
        // TODO only seed with u64 for now because serde_yaml doesn't serialize u128 correctly
        let seed = match seed {
            Some(s) => s,
            None => thread_rng().gen::<u64>() as u128,
        };

        ReproducibleRandom {
            seed,
            gen: Pcg64Mcg::new(seed),
        }
    }

    pub fn gen<T: SampleUniform + Sized>(&mut self, min: T, max: T) -> T {
        self.gen.gen_range(min, max)
    }

    pub fn shuffle<T>(&mut self, values: &mut [T]) {
        values.shuffle(&mut self.gen);
    }

    pub fn seed(&self) -> u128 {
        self.seed
    }
}

impl std::fmt::Debug for ReproducibleRandom {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let state = serde_json::to_string(&self.gen).map_err(|_| std::fmt::Error)?;
        write!(f, "Random: {}", state)
    }
}

pub fn shuffle<T>(values: &mut [T]) {
    values.shuffle(&mut thread_rng());
}

pub fn gen_rand<T: SampleUniform + Sized>(min: T, max: T) -> T {
    thread_rng().gen_range(min, max)
}

pub fn gen_rand_weight<'a, T, V>(items: &'a [(T, V)]) -> &'a T
where
    V: Sized + Clone + Default + PartialOrd + Copy,
    V: for<'b> std::ops::AddAssign<&'b V>,
    V: SampleUniform,
{
    let dist = WeightedIndex::new(items.iter().map(|item| item.1)).unwrap();
    &items[dist.sample(&mut thread_rng())].0
}

fn active_resources_file_path() -> PathBuf {
    let mut path = config::USER_DIR.clone();
    path.push("active_resources.yml");
    path
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ActiveResources {
    pub campaign: Option<String>,
    pub mods: Vec<String>,
}

impl ActiveResources {
    pub fn read() -> ActiveResources {
        let path = active_resources_file_path();

        let data = match fs::read_to_string(path) {
            Ok(data) => data,
            Err(_) => {
                info!("active_resources file not found");
                return ActiveResources::default();
            }
        };

        let active_resources: ActiveResources = match serde_yaml::from_str(&data) {
            Ok(val) => val,
            Err(e) => {
                warn!("Error reading active resources file");
                warn!("{}", e);
                return ActiveResources::default();
            }
        };

        active_resources
    }

    pub fn write(&self) {
        let file = active_resources_file_path();
        match write_to_file(file, self) {
            Ok(()) => (),
            Err(e) => {
                warn!("Error writing active resources file");
                warn!("{}", e);
            }
        }
    }

    pub fn directories(&self) -> Vec<String> {
        let mut dirs = Vec::new();

        dirs.push(Config::resources_config().directory);
        if let Some(ref dir) = self.campaign {
            dirs.push(dir.to_string());
        }

        for mod_dir in self.mods.iter() {
            dirs.push(mod_dir.to_string());
        }

        dirs
    }
}

impl Default for ActiveResources {
    fn default() -> Self {
        ActiveResources {
            campaign: None,
            mods: Vec::new(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields, untagged)]
pub enum ExtInt {
    Int(u32),
    Infinity,
}

impl Ord for ExtInt {
    fn cmp(&self, other: &ExtInt) -> Ordering {
        match self {
            ExtInt::Int(val) => match other {
                ExtInt::Int(other) => val.cmp(&other),
                ExtInt::Infinity => Ordering::Less,
            },
            ExtInt::Infinity => match other {
                ExtInt::Int(_) => Ordering::Greater,
                ExtInt::Infinity => Ordering::Equal,
            },
        }
    }
}

impl PartialOrd for ExtInt {
    fn partial_cmp(&self, other: &ExtInt) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl ExtInt {
    pub fn max(a: ExtInt, b: ExtInt) -> ExtInt {
        if a > b {
            a
        } else {
            b
        }
    }

    pub fn min(a: ExtInt, b: ExtInt) -> ExtInt {
        if a > b {
            b
        } else {
            a
        }
    }

    pub fn divide(self, other: ExtInt) -> f32 {
        match self {
            ExtInt::Int(amount) => match other {
                ExtInt::Int(other_amount) => amount as f32 / other_amount as f32,
                ExtInt::Infinity => 0.0,
            },
            ExtInt::Infinity => match other {
                ExtInt::Int(_) => 0.0,
                ExtInt::Infinity => 1.0,
            },
        }
    }

    pub fn is_zero(self) -> bool {
        match self {
            ExtInt::Int(amount) => amount == 0,
            ExtInt::Infinity => false,
        }
    }

    pub fn is_infinite(self) -> bool {
        match self {
            ExtInt::Int(_) => false,
            ExtInt::Infinity => true,
        }
    }

    pub fn to_f32(self) -> f32 {
        match self {
            ExtInt::Int(amount) => amount as f32,
            ExtInt::Infinity => 1e12, // use a value that serde json can serialize properly
        }
    }

    pub fn less_than(self, other: u32) -> bool {
        match self {
            ExtInt::Int(amount) => amount < other,
            ExtInt::Infinity => false,
        }
    }

    pub fn greater_than(self, other: u32) -> bool {
        match self {
            ExtInt::Int(amount) => amount > other,
            ExtInt::Infinity => true,
        }
    }
}

impl fmt::Display for ExtInt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ExtInt::Int(amount) => write!(f, "{}", amount),
            ExtInt::Infinity => write!(f, "infinity"),
        }
    }
}

impl Mul<u32> for ExtInt {
    type Output = ExtInt;
    fn mul(self, other: u32) -> ExtInt {
        match self {
            ExtInt::Int(amount) => ExtInt::Int(amount * other),
            ExtInt::Infinity => ExtInt::Infinity,
        }
    }
}

impl Add<ExtInt> for ExtInt {
    type Output = ExtInt;
    fn add(self, other: ExtInt) -> ExtInt {
        match self {
            ExtInt::Int(amount) => match other {
                ExtInt::Int(other_amount) => ExtInt::Int(amount + other_amount),
                ExtInt::Infinity => ExtInt::Infinity,
            },
            ExtInt::Infinity => ExtInt::Infinity,
        }
    }
}

impl Add<u32> for ExtInt {
    type Output = ExtInt;
    fn add(self, other: u32) -> ExtInt {
        match self {
            ExtInt::Int(amount) => ExtInt::Int(amount + other),
            ExtInt::Infinity => ExtInt::Infinity,
        }
    }
}

impl Sub<u32> for ExtInt {
    type Output = ExtInt;
    fn sub(self, other: u32) -> ExtInt {
        match self {
            ExtInt::Int(amount) => {
                if other > amount {
                    ExtInt::Int(0)
                } else {
                    ExtInt::Int(amount - other)
                }
            }
            ExtInt::Infinity => ExtInt::Infinity,
        }
    }
}

pub fn invalid_data_error<T>(str: &str) -> Result<T, Error> {
    Err(Error::new(ErrorKind::InvalidData, str))
}

pub fn unable_to_create_error<T>(kind: &str, id: &str) -> Result<T, Error> {
    Err(Error::new(
        ErrorKind::InvalidData,
        format!("Unable to create {} '{}'", kind, id),
    ))
}

/// Helper function to return the number of milliseconds elapsed in
/// the given duration.
pub fn get_elapsed_millis(elapsed: Duration) -> u32 {
    (elapsed.as_secs() as u32) * 1_000 + elapsed.subsec_millis()
}

/// Helper function to return a string representation of the elapsed time
/// in seconds
pub fn format_elapsed_secs(elapsed: Duration) -> String {
    let secs = elapsed.as_secs() as f64 + elapsed.subsec_nanos() as f64 * 1e-9;
    format!("{:.6}", secs)
}

pub fn ok_and_exit(message: &str) {
    info!("{}", message);
    info!("Exiting...");
    ::std::process::exit(0)
}

pub fn error_and_exit(error: &str) {
    error!("{}", error);
    error!("Exiting...");
    ::std::process::exit(1)
}

pub fn main_loop(
    system: &mut System,
    root: Rc<RefCell<Widget>>,
    updater: Box<dyn MainLoopUpdater>,
) -> Result<(), Error> {
    let fpms = (1000.0 / (Config::frame_rate() as f32)) as u64;
    let frame_time = time::Duration::from_millis(fpms);
    trace!("Computed {} frames per milli.", fpms);

    info!("Starting main loop.");
    let main_loop_start_time = time::Instant::now();

    let mut frames = 0;
    let mut render_time = time::Duration::from_secs(0);
    let mut last_start_time = time::Instant::now();

    loop {
        let last_elapsed = get_elapsed_millis(last_start_time.elapsed());
        last_start_time = time::Instant::now();
        let total_elapsed = get_elapsed_millis(main_loop_start_time.elapsed());

        system.io().process_input(Rc::clone(&root));
        updater.update(&root, last_elapsed);

        Audio::update(system.audio(), last_elapsed);

        if let Err(e) = Widget::update(&root, last_elapsed) {
            error!("There was a fatal error updating the UI tree state.");
            return Err(e);
        }

        system.io().render_output(root.borrow(), total_elapsed);

        if updater.is_exit() {
            trace!("Exiting main loop.");
            break;
        }

        let frame_elapsed = last_start_time.elapsed();
        if frame_time > frame_elapsed {
            thread::sleep(frame_time - frame_elapsed);
        }

        render_time += frame_elapsed;
        frames += 1;
    }

    let secs = render_time.as_secs() as f64 + render_time.subsec_nanos() as f64 * 1e-9;
    info!(
        "Rendered {} frames with total render time {:.4} seconds",
        frames, secs
    );
    info!(
        "Average frame render time: {:.2} milliseconds",
        1000.0 * secs / frames as f64
    );

    Ok(())
}

pub fn setup_logger() {
    let mut path = config::USER_DIR.clone();
    path.push("log");
    let log_dir = path;

    let log_config = Config::logging_config();

    let mut log_builder = LogSpecBuilder::new();
    log_builder.default(log_config.log_level);

    let dup = match log_config.stderr_log_level {
        LevelFilter::Error => Duplicate::Error,
        LevelFilter::Warn => Duplicate::Warn,
        LevelFilter::Info => Duplicate::Info,
        LevelFilter::Debug => Duplicate::Debug,
        LevelFilter::Trace => Duplicate::Trace,
        LevelFilter::Off => Duplicate::None,
    };

    let mut logger = Logger::with(log_builder.finalize())
        .log_to_file()
        .print_message()
        .directory(log_dir)
        .duplicate_to_stderr(dup)
        .format(opt_format);

    if log_config.append {
        logger = logger.append();
    }

    if !log_config.use_timestamps {
        logger = logger.suppress_timestamp();
    }

    logger.start().unwrap_or_else(|e| {
        eprintln!("{}", e);
        eprintln!("There was a fatal error initializing logging to 'log/'");
        eprintln!("Exiting...");
        ::std::process::exit(1);
    });

    panic::set_hook(Box::new(|p| {
        error!("Thread main panic.");
        warn!("with payload: {:?}", p.payload());
        if let Some(loc) = p.location() {
            warn!("at {:?}", loc);
        }

        let bt = Backtrace::new();
        warn!("{:?}", bt);
    }));

    create_user_dirs();
}

fn create_user_dirs() {
    let res = Config::resources_config();

    let mut campaign_dir = config::USER_DIR.clone();
    campaign_dir.push(&res.campaigns_directory);
    config::create_dir_and_warn(&campaign_dir);

    let mut mods_dir = config::USER_DIR.clone();
    mods_dir.push(&res.mods_directory);
    config::create_dir_and_warn(&mods_dir);
}

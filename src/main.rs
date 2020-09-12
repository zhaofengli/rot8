extern crate clap;
extern crate glob;
extern crate regex;

use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use clap::{App, Arg};
use glob::glob;
use serde::Deserialize;
use serde_json::Value;

enum Backend {
    Sway,
    Xorg,
}

enum KeyboardMode {
    Integrated,
    Detachable,
    None,
}

#[derive(Deserialize)]
struct SwayOutput {
    name: String,
    transform: String,
}

fn get_keyboards(backend: &Backend) -> Result<Vec<String>, String> {
    match backend {
        Backend::Sway => {
            let raw_inputs = String::from_utf8(
                Command::new("swaymsg")
                    .arg("-t")
                    .arg("get_inputs")
                    .arg("--raw")
                    .output()
                    .expect("Swaymsg get inputs command failed")
                    .stdout,
            )
            .unwrap();

            let mut keyboards = vec![];
            let deserialized: Vec<Value> = serde_json::from_str(&raw_inputs)
                .expect("Unable to deserialize swaymsg JSON output");
            for output in deserialized {
                let input_type = output["type"].as_str().unwrap();
                if input_type == "keyboard" {
                    keyboards.push(output["identifier"].to_string());
                }
            }

            return Ok(keyboards);
        }
        Backend::Xorg => {
            return Ok(vec![]);
        }
    }
}

fn keyboards_attached<T: AsRef<std::ffi::OsStr>>(backend: &Backend, keyboards: &[T]) -> bool {
    match backend {
        Backend::Sway => {
            // TODO
            false
        }
        Backend::Xorg => {
            for keyboard in keyboards {
                let probe = Command::new("xinput")
                    .arg("list")
                    .arg(&keyboard)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .expect("Xinput list command failed to start");

                if probe.success() {
                    return true;
                }
            }
            return false;
        }
    }
}

fn get_window_server_rotation_state(display: &str, backend: &Backend) -> Result<String, String> {
    match backend {
        Backend::Sway => {
            let raw_rotation_state = String::from_utf8(
                Command::new("swaymsg")
                    .arg("-t")
                    .arg("get_outputs")
                    .arg("--raw")
                    .output()
                    .expect("Swaymsg get outputs command failed to start")
                    .stdout,
            )
            .unwrap();
            let deserialized: Vec<SwayOutput> = serde_json::from_str(&raw_rotation_state)
                .expect("Unable to deserialize swaymsg JSON output");
            for output in deserialized {
                if output.name == display {
                    return Ok(output.transform);
                }
            }

            return Err(format!(
                "Unable to determine rotation state: display {} not found in 'swaymsg -t get_outputs'",
                display
            )
                .to_owned());
        }
        Backend::Xorg => {
            let raw_rotation_state = String::from_utf8(
                Command::new("xrandr")
                    .output()
                    .expect("Xrandr get outputs command failed to start")
                    .stdout,
            )
            .unwrap();
            let xrandr_output_pattern = regex::Regex::new(format!(
                r"^{} connected .+? .+? (normal |inverted |left |right )?\(normal left inverted right x axis y axis\) .+$",
                regex::escape(display),
            ).as_str()).unwrap();
            for xrandr_output_line in raw_rotation_state.split("\n") {
                if !xrandr_output_pattern.is_match(xrandr_output_line) {
                    continue;
                }

                let xrandr_output_captures =
                    xrandr_output_pattern.captures(xrandr_output_line).unwrap();
                if let Some(transform) = xrandr_output_captures.get(1) {
                    return Ok(transform.as_str().to_owned());
                } else {
                    return Ok("normal".to_owned());
                }
            }

            return Err(format!(
                "Unable to determine rotation state: display {} not found in xrandr output",
                display
            )
            .to_owned());
        }
    }
}

fn get_scale() -> Option<f32> {
    match glob("/sys/bus/iio/devices/iio:device*/in_accel_scale") {
        Ok(mut paths) => {
            let path = paths.next()?.ok()?;
            let scale_raw = fs::read_to_string(path).ok()?;
            let scale = scale_raw.trim_end_matches('\n').parse::<f32>().ok()?;
            Some(scale)
        }
        Err(_) => None,
    }
}

#[derive(Debug)]
struct Orientation {
    vector: (f32, f32),
    new_state: &'static str,
    x_state: &'static str,
    matrix: [&'static str; 9],
}

fn main() -> Result<(), String> {
    let mut new_state: &str;
    let mut x_state: &str;

    let mut path_x: String = "".to_string();
    let mut path_y: String = "".to_string();
    let mut matrix: [&str; 9];

    let backend = if String::from_utf8(Command::new("pidof").arg("sway").output().unwrap().stdout)
        .unwrap()
        .len()
        >= 1
    {
        Backend::Sway
    } else if String::from_utf8(Command::new("pidof").arg("Xorg").output().unwrap().stdout)
        .unwrap()
        .len()
        >= 1
    {
        Backend::Xorg
    } else {
        return Err("Unable to find Sway or Xorg procceses".to_owned());
    };

    let args = vec![
        Arg::with_name("sleep")
            .default_value("500")
            .long("sleep")
            .short("s")
            .value_name("SLEEP")
            .help("Set sleep millis")
            .takes_value(true),
        Arg::with_name("display")
            .default_value("eDP-1")
            .long("display")
            .short("d")
            .value_name("DISPLAY")
            .help("Set Display Device")
            .takes_value(true),
        Arg::with_name("touchscreen")
            .default_value("ELAN0732:00 04F3:22E1")
            .long("touchscreen")
            .short("i")
            .value_name("TOUCHSCREEN")
            .help("Set Touchscreen input Device (X11 only)")
            .takes_value(true),
        Arg::with_name("threshold")
            .default_value("0.5")
            .long("threshold")
            .short("t")
            .value_name("THRESHOLD")
            .help("Set a rotation threshold between 0 and 1")
            .takes_value(true),

        Arg::with_name("keyboard_mode")
            .default_value("integrated")
            .long("keyboard-mode")
            .value_name("KEYBOARD_MODE")
            .help(
                "'integrated' - The keyboard is an integral part of the device. Disable it when device is rotated (Sway only).\n\
                'detachable' - The keyboard is detachable. Lock the rotation when it's attached.\n\
                'none' - Do not enable/disable keyboard"
            )
            .takes_value(true),
        Arg::with_name("keyboard_device")
            .long("keyboard")
            .value_name("KEYBOARD_DEVICE")
            .help("Set keyboard device")
            .takes_value(true),

        // PineTab Hack
        Arg::with_name("rotate_90")
            .long("rotate-90")
            .value_name("ROTATE_90")
            .help("[PineTab Hack] Enable if the content is 90 degrees counterclockwise when upright")
            .takes_value(false),
        Arg::with_name("flip_y")
            .long("flip-y")
            .value_name("FLIP_Y")
            .help("[PineTab Hack] Flip Y axis")
            .takes_value(false),

        Arg::with_name("rotate_hook")
            .long("rotate-hook")
            .value_name("ROTATE_HOOK")
            .help("A shell command to run after rotation")
            .takes_value(true),
    ];

    let cmd_lines = App::new("rot8").version("0.1.3").args(&args);

    let matches = cmd_lines.get_matches();

    let sleep = matches.value_of("sleep").unwrap_or("default.conf");
    let display = matches.value_of("display").unwrap_or("default.conf");
    let touchscreen = matches.value_of("touchscreen").unwrap_or("default.conf");
    let threshold = matches.value_of("threshold").unwrap_or("default.conf");
    let old_state_owned = get_window_server_rotation_state(display, &backend)?;
    let mut old_state = old_state_owned.as_str();

    let keyboard_mode = match matches.value_of("keyboard_mode") {
        Some("integrated") => KeyboardMode::Integrated,
        Some("detachable") => KeyboardMode::Detachable,
        Some("none") => KeyboardMode::None,
        _ => panic!("--keyboard-mode can be one of 'integrated', 'detachable', and 'none'"),
    };

    let keyboards = if matches.is_present("keyboard_device") {
        vec![String::from(matches.value_of("keyboard_device").unwrap())]
    } else {
        get_keyboards(&backend)?
    };

    // PineTab Hack
    let rotate_90 = matches.is_present("rotate_90");
    let flip_y = matches.is_present("flip_y");

    let rotate_hook = matches.value_of("rotate_hook");

    let scale = get_scale();

    for entry in glob("/sys/bus/iio/devices/iio:device*/in_accel_*_raw").unwrap() {
        match entry {
            Ok(path) => {
                if path.to_str().unwrap().contains("x_raw") {
                    path_x = path.to_str().unwrap().to_owned();
                } else if path.to_str().unwrap().contains("y_raw") {
                    path_y = path.to_str().unwrap().to_owned();
                } else if path.to_str().unwrap().contains("z_raw") {
                    continue;
                } else {
                    panic!("Unknown accelerometer device path {:?}", path);
                }
            }
            Err(e) => println!("{:?}", e),
        }
    }

    let orientations = [
        Orientation {
            vector: (0.0, -1.0),
            new_state: "normal",
            x_state: "normal",
            matrix: ["1", "0", "0", "0", "1", "0", "0", "0", "1"],
        },
        Orientation {
            vector: (0.0, 1.0),
            new_state: "180",
            x_state: "inverted",
            matrix: ["-1", "0", "1", "0", "-1", "1", "0", "0", "1"],
        },
        Orientation {
            vector: (-1.0, 0.0),
            new_state: "90",
            x_state: "right",
            matrix: ["0", "1", "0", "-1", "0", "1", "0", "0", "1"],
        },
        Orientation {
            vector: (1.0, 0.0),
            new_state: "270",
            x_state: "left",
            matrix: ["0", "-1", "1", "1", "0", "0", "0", "0", "1"],
        },
    ];

    let mut current_orient: &Orientation = &orientations[0];

    loop {
        let x_raw = fs::read_to_string(path_x.as_str()).unwrap();
        let y_raw = fs::read_to_string(path_y.as_str()).unwrap();
        let x_clean: f32 = x_raw.trim_end_matches('\n').parse::<i32>().unwrap_or(0) as f32;
        let mut y_clean: f32 = y_raw.trim_end_matches('\n').parse::<i32>().unwrap_or(0) as f32;

        let human_normal = if rotate_90 {
            "90"
        } else {
            "normal"
        };

        if flip_y {
            y_clean = -y_clean;
        }

        // Normalize vectors
        let (mut x, mut y): (f32, f32) = match scale {
            Some(scale) => {
                (x_clean * scale / 10f32, y_clean * scale / 10f32)
            }
            None => (x_clean / 1f32, y_clean / 1f32),
        };


        // Rotate (HACK)
        if rotate_90 {
            // Rotate 90deg clockwise
            let mx = -x;
            x = y;
            y = mx;
        }

        for (_i, orient) in orientations.iter().enumerate() {
            let d = (x - orient.vector.0).powf(2.0) + (y - orient.vector.1).powf(2.0);

            if d < threshold.parse::<f32>().unwrap_or(0.5) {
                current_orient = &orient;
                break;
            }
        }

        new_state = current_orient.new_state;
        x_state = current_orient.x_state;
        matrix = current_orient.matrix;

        if new_state != old_state {
            let integrated_keyboard_state = if new_state == human_normal {
                "enabled"
            } else {
                "disabled"
            };

            println!("{} -> {} (human_normal is {})", old_state, new_state, human_normal);
            let noop = if let KeyboardMode::Detachable = keyboard_mode {
                // If there are keyboards attached, refuse to rotate to
                // any orientation but human_normal
                keyboards_attached(&backend, &keyboards) &&
                (old_state == human_normal || new_state != human_normal)
            } else {
                false
            };

            if !noop {
                match backend {
                    Backend::Sway => {
                        Command::new("swaymsg")
                            .arg("output")
                            .arg(display)
                            .arg("transform")
                            .arg(new_state)
                            .spawn()
                            .expect("Swaymsg rotate command failed to start")
                            .wait()
                            .expect("Swaymsg rotate command wait failed");

                        if let KeyboardMode::Integrated = keyboard_mode {
                            // Disable integrated keyboard when not human_normal
                            for keyboard in &keyboards {
                                Command::new("swaymsg")
                                    .arg("input")
                                    .arg(keyboard)
                                    .arg("events")
                                    .arg(integrated_keyboard_state)
                                    .spawn()
                                    .expect("Swaymsg keyboard command failed to start")
                                    .wait()
                                    .expect("Swaymsg keyboard command wait failed");
                            }
                        }
                    }
                    Backend::Xorg => {
                        Command::new("xrandr")
                            .arg("--output")
                            .arg(display)
                            .arg("--rotate")
                            .arg(x_state)
                            .spawn()
                            .expect("Xrandr rotate command failed to start")
                            .wait()
                            .expect("Xrandr rotate command wait failed");

                        Command::new("xinput")
                            .arg("set-prop")
                            .arg(touchscreen)
                            .arg("Coordinate Transformation Matrix")
                            .args(&matrix)
                            .spawn()
                            .expect("Xinput rotate command failed to start")
                            .wait()
                            .expect("Xinput rotate command wait failed");

                    }
                }
                if let Some(hook) = rotate_hook {
                    Command::new("/bin/sh")
                        .arg("-c")
                        .arg(hook)
                        .spawn()
                        .expect("Rotate hook command failed to start")
                        .wait()
                        .expect("Rotate hook command wait failed");
                }
            }
            old_state = new_state;
        }
        thread::sleep(Duration::from_millis(sleep.parse::<u64>().unwrap_or(0)));
    }
}

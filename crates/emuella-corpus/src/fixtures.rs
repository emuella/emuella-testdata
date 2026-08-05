use crate::CatalogueError;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const GENERATED_CORE_ID: &str = "common/generated-core";

const README: &str = r#"# Generated core image pack

These fixtures were generated deterministically by `emuella-corpus` from
Emuella-authored algorithms. They are licensed under Apache-2.0 with the
catalogue software and are safe to copy into Apache-2.0 test distributions.

The pack intentionally uses simple, independently inspectable formats:

- PGM/PPM for unsigned 8- and 16-bit integer samples;
- PAM for RGBA frames;
- PGX for signed 12-bit JPEG 2000 component samples; and
- PFM for little-endian scene-linear floating-point RGB.

File headers are part of the fixture bytes. Canonical sample semantics are
recorded in the pack manifest; codec tests should distinguish source-format
parsing from codec behavior when reporting failures.
"#;

const PROVENANCE: &str = r#"schema_version = 1
pack_id = "common/generated-core"
pack_version = "1"
generator = "emuella-corpus generated-core-v1"
random_algorithm = "xorshift64"
random_seed = "0x6a09e667f3bcc909"
timestamped = false
license = "Apache-2.0"
"#;

pub fn generate_pack(id: &str, output: &Path) -> Result<Vec<PathBuf>, CatalogueError> {
    if id != GENERATED_CORE_ID {
        return Err(CatalogueError::message(format!(
            "pack {id} has no built-in generator"
        )));
    }
    ensure_empty_output(output)?;
    fs::create_dir_all(output).map_err(|error| io_error("create", output, error))?;

    let mut written = Vec::new();
    write_text(output, "README.md", README, &mut written)?;
    write_text(output, "PROVENANCE.toml", PROVENANCE, &mut written)?;

    write_pgm_u8(output, "gray-u8-1x1.pgm", 1, 1, |_x, _y| 0, &mut written)?;
    write_pgm_u8(
        output,
        "gray-u8-prime-gradient-17x19.pgm",
        17,
        19,
        |x, y| ((x * 13 + y * 29 + x * y * 3) & 0xff) as u8,
        &mut written,
    )?;
    write_pgm_u8(
        output,
        "gray-u8-checkerboard-127x131.pgm",
        127,
        131,
        |x, y| if ((x / 3) + (y / 5)) & 1 == 0 { 0 } else { 255 },
        &mut written,
    )?;
    write_ppm_u8(
        output,
        "rgb-u8-edges-31x29.ppm",
        31,
        29,
        |x, y| {
            let red = if x < 15 { 16 } else { 240 };
            let green = ((x * 17 + y * 7) & 0xff) as u8;
            let blue = if x == y || x + y == 30 { 255 } else { 0 };
            [red, green, blue]
        },
        &mut written,
    )?;
    write_pam_rgba_u8(
        output,
        "rgba-u8-alpha-67x61.pam",
        67,
        61,
        |x, y| {
            let alpha = ((x * 255) / 66) as u8;
            [
                ((x * 11 + y * 3) & 0xff) as u8,
                ((y * 19) & 0xff) as u8,
                if (x + y) & 1 == 0 { 255 } else { 32 },
                alpha,
            ]
        },
        &mut written,
    )?;
    write_pgm_u16(
        output,
        "gray-u16-ramp-257x193.pgm",
        257,
        193,
        |x, y| ((x * 257 + y * 911 + x * y) & 0xffff) as u16,
        &mut written,
    )?;
    write_ppm_u16(
        output,
        "rgb-u16-extrema-65x63.ppm",
        65,
        63,
        |x, y| {
            let selector = (x + 2 * y) % 5;
            [
                if selector == 0 { 0 } else { 65_535 },
                ((x * 997 + y * 313) & 0xffff) as u16,
                if selector == 4 { 65_535 } else { 1 },
            ]
        },
        &mut written,
    )?;
    write_pgx_i12(
        output,
        "signed-12bit-impulse-71x69.pgx",
        71,
        69,
        |x, y| {
            if (x, y) == (0, 0) {
                -2048
            } else if (x, y) == (70, 68) {
                2047
            } else if x == 35 || y == 34 {
                1024 - (((x + y) & 0x7ff) as i16)
            } else {
                0
            }
        },
        &mut written,
    )?;
    write_pfm_rgb_f32(
        output,
        "rgb-f32-linear-hdr-73x59.pfm",
        73,
        59,
        |x, y| {
            let xf = x as f32 / 72.0;
            let yf = y as f32 / 58.0;
            [xf * xf * 16.0, yf * 4.0, (xf + yf) * 0.625 - 0.25]
        },
        &mut written,
    )?;
    write_pgm_u8(
        output,
        "gray-u8-wide-4097x3.pgm",
        4097,
        3,
        |x, y| ((x * 37 + y * 101) & 0xff) as u8,
        &mut written,
    )?;
    write_pgm_u8(
        output,
        "gray-u8-tall-3x4099.pgm",
        3,
        4099,
        |x, y| ((x * 103 + y * 41) & 0xff) as u8,
        &mut written,
    )?;

    let mut random = XorShift64::new(0x6a09_e667_f3bc_c909);
    write_pgm_u8(
        output,
        "gray-u8-noise-257x263.pgm",
        257,
        263,
        |_x, _y| random.next_u8(),
        &mut written,
    )?;

    for frame in 0..4_u32 {
        let name = format!("animation/frame-{frame:02}.pam");
        write_pam_rgba_u8(
            output,
            &name,
            96,
            64,
            |x, y| {
                let center_x = 16 + frame * 20;
                let dx = x.abs_diff(center_x);
                let dy = y.abs_diff(32);
                let inside = dx * dx + dy * dy <= 12 * 12;
                [
                    if inside { 255 } else { (x * 2) as u8 },
                    if inside {
                        (frame * 70) as u8
                    } else {
                        (y * 3) as u8
                    },
                    if inside { 32 } else { 128 },
                    if inside { 192 } else { ((x + y) & 0x3f) as u8 },
                ]
            },
            &mut written,
        )?;
    }

    written.sort();
    Ok(written)
}

fn ensure_empty_output(output: &Path) -> Result<(), CatalogueError> {
    match fs::read_dir(output) {
        Ok(mut entries) => {
            if entries
                .next()
                .transpose()
                .map_err(|error| io_error("inspect", output, error))?
                .is_some()
            {
                return Err(CatalogueError::message(format!(
                    "generation output is not empty: {}",
                    output.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error("inspect", output, error)),
    }
    Ok(())
}

fn write_text(
    output: &Path,
    relative: &str,
    contents: &str,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError> {
    let path = output.join(relative);
    prepare_parent(&path)?;
    fs::write(&path, contents.as_bytes()).map_err(|error| io_error("write", &path, error))?;
    written.push(PathBuf::from(relative));
    Ok(())
}

fn write_pgm_u8<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> u8,
{
    write_binary(output, relative, written, |file| {
        write!(file, "P5\n{width} {height}\n255\n")?;
        for y in 0..height {
            for x in 0..width {
                file.write_all(&[sample(x, y)])?;
            }
        }
        Ok(())
    })
}

fn write_ppm_u8<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> [u8; 3],
{
    write_binary(output, relative, written, |file| {
        write!(file, "P6\n{width} {height}\n255\n")?;
        for y in 0..height {
            for x in 0..width {
                file.write_all(&sample(x, y))?;
            }
        }
        Ok(())
    })
}

fn write_pam_rgba_u8<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> [u8; 4],
{
    write_binary(output, relative, written, |file| {
        write!(
            file,
            "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
        )?;
        for y in 0..height {
            for x in 0..width {
                file.write_all(&sample(x, y))?;
            }
        }
        Ok(())
    })
}

fn write_pgm_u16<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> u16,
{
    write_binary(output, relative, written, |file| {
        write!(file, "P5\n{width} {height}\n65535\n")?;
        for y in 0..height {
            for x in 0..width {
                file.write_all(&sample(x, y).to_be_bytes())?;
            }
        }
        Ok(())
    })
}

fn write_ppm_u16<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> [u16; 3],
{
    write_binary(output, relative, written, |file| {
        write!(file, "P6\n{width} {height}\n65535\n")?;
        for y in 0..height {
            for x in 0..width {
                for component in sample(x, y) {
                    file.write_all(&component.to_be_bytes())?;
                }
            }
        }
        Ok(())
    })
}

fn write_pgx_i12<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> i16,
{
    write_binary(output, relative, written, |file| {
        writeln!(file, "PG ML -12 {width} {height}")?;
        for y in 0..height {
            for x in 0..width {
                let value = sample(x, y);
                if !(-2048..=2047).contains(&value) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "generated PGX value is outside signed 12-bit range",
                    ));
                }
                file.write_all(&value.to_be_bytes())?;
            }
        }
        Ok(())
    })
}

fn write_pfm_rgb_f32<F>(
    output: &Path,
    relative: &str,
    width: u32,
    height: u32,
    mut sample: F,
    written: &mut Vec<PathBuf>,
) -> Result<(), CatalogueError>
where
    F: FnMut(u32, u32) -> [f32; 3],
{
    write_binary(output, relative, written, |file| {
        write!(file, "PF\n{width} {height}\n-1.0\n")?;
        for y in (0..height).rev() {
            for x in 0..width {
                for component in sample(x, y) {
                    file.write_all(&component.to_le_bytes())?;
                }
            }
        }
        Ok(())
    })
}

fn write_binary<F>(
    output: &Path,
    relative: &str,
    written: &mut Vec<PathBuf>,
    writer: F,
) -> Result<(), CatalogueError>
where
    F: FnOnce(&mut fs::File) -> io::Result<()>,
{
    let path = output.join(relative);
    prepare_parent(&path)?;
    let mut file = fs::File::create(&path).map_err(|error| io_error("create", &path, error))?;
    writer(&mut file).map_err(|error| io_error("write", &path, error))?;
    file.flush()
        .map_err(|error| io_error("flush", &path, error))?;
    written.push(PathBuf::from(relative));
    Ok(())
}

fn prepare_parent(path: &Path) -> Result<(), CatalogueError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error("create", parent, error))?;
    }
    Ok(())
}

fn io_error(action: &str, path: &Path, error: io::Error) -> CatalogueError {
    CatalogueError::message(format!("failed to {action} {}: {error}", path.display()))
}

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u8(&mut self) -> u8 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        (value >> 56) as u8
    }
}

// Kompiluje rdzen wyszukiwania (C++) i osadza manifest (DPI per-monitor v2,
// Common Controls 6) w pliku exe.
fn main() {
    println!("cargo:rerun-if-changed=cpp/search.cpp");
    println!("cargo:rerun-if-changed=cpp/search.h");
    println!("cargo:rerun-if-changed=indeks.manifest");

    cc::Build::new()
        .cpp(true)
        .file("cpp/search.cpp")
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/O2")
        .flag_if_supported("/utf-8")
        .warnings(true)
        .compile("indeks_search");

    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("indeks.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());

    build_overlay_engine();
}

/// Silnik nakladki w C# -> indeks-nakladka.exe obok indeks.exe. Kompilator csc z .NET
/// Framework 4 jest w kazdym Windows 10/11, wiec nie trzeba instalowac SDK.
fn build_overlay_engine() {
    println!("cargo:rerun-if-changed=csharp/Nakladka.cs");
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let csc = std::path::Path::new(&windir).join(r"Microsoft.NET\Framework64\v4.0.30319\csc.exe");
    if !csc.exists() {
        println!("cargo:warning=brak csc.exe (.NET Framework 4) - nakladka zostaje w Rust");
        return;
    }
    // OUT_DIR = target\<profil>\build\indeks-xxxx\out -> target\<profil>
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let Some(target) = out_dir.ancestors().nth(3) else { return };
    let exe = target.join("indeks-nakladka.exe");
    let out = std::process::Command::new(&csc)
        .args(["/nologo", "/target:winexe", "/optimize+", "/platform:x64", "/r:System.Windows.Forms.dll", "/r:System.Drawing.dll"])
        .arg(format!("/out:{}", exe.display()))
        .arg(std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("csharp").join("Nakladka.cs"))
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("error")) {
                println!("cargo:warning=csc: {}", line);
            }
            println!("cargo:warning=kompilacja csharp/Nakladka.cs nie powiodla sie - nakladka zostaje w Rust");
        }
        Err(e) => println!("cargo:warning=csc: {}", e),
    }
}

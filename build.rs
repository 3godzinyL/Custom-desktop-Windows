// Kompiluje rdzen wyszukiwania (C++) i osadza manifest (DPI per-monitor v2,
// Common Controls 6) w pliku exe.
fn main() {
    println!("cargo:rerun-if-changed=cpp/search.cpp");
    println!("cargo:rerun-if-changed=cpp/search.h");
    println!("cargo:rerun-if-changed=cpp/optymalizator.cpp");
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
    build_panel_engine();
    build_catalog_engine();
    build_optimizer_engine();
}

/// Silnik optymalizacji w C -> osobny indeks-optymalizator.exe (nie wkompilowany w Rust):
/// czarna robota (próbkowanie + dławienie EcoQoS/priorytet/RAM), gada z Rustem przez potok.
fn build_optimizer_engine() {
    println!("cargo:rerun-if-changed=cpp/optymalizator.cpp");
    let Some(target) = target_dir() else { return };
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let objs = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let compiler = cc::Build::new().cpp(true).get_compiler();
    let out = compiler
        .to_command()
        .args(["/nologo", "/std:c++17", "/EHsc", "/O2", "/utf-8", "/MT"])
        .arg(root.join("cpp").join("optymalizator.cpp"))
        .arg(format!("/Fo{}\\", objs.display()))
        .arg(format!("/Fe{}", target.join("indeks-optymalizator.exe").display()))
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("error")) {
                println!("cargo:warning=cl: {}", line);
            }
            panic!("kompilacja silnika optymalizacji (cpp/optymalizator.cpp) nie powiodla sie");
        }
        Err(e) => panic!("cl: {}", e),
    }
}

/// Katalog obok indeks.exe: target\<profil> (OUT_DIR = target\<profil>\build\indeks-xxxx\out).
fn target_dir() -> Option<std::path::PathBuf> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    out_dir.ancestors().nth(3).map(|p| p.to_path_buf())
}

/// Silnik katalogu w C++ -> indeks-katalog.exe. Osobny proces: przeszukuje dysk i konczy sie,
/// wiec dane skanu nie zostaja w pamieci pulpitu.
fn build_catalog_engine() {
    for f in ["cpp/katalog.cpp", "cpp/katalog_skan.cpp", "cpp/katalog.h"] {
        println!("cargo:rerun-if-changed={}", f);
    }
    let Some(target) = target_dir() else { return };
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let objs = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let compiler = cc::Build::new().cpp(true).get_compiler();
    let out = compiler
        .to_command()
        .args(["/nologo", "/std:c++17", "/EHsc", "/O2", "/utf-8", "/MT", "/DUNICODE", "/D_UNICODE"])
        .arg(root.join("cpp").join("katalog.cpp"))
        .arg(root.join("cpp").join("katalog_skan.cpp"))
        .arg(format!("/Fo{}\\", objs.display()))
        .arg(format!("/Fe{}", target.join("indeks-katalog.exe").display()))
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("error")) {
                println!("cargo:warning=cl: {}", line);
            }
            panic!("kompilacja silnika katalogu (cpp/katalog*.cpp) nie powiodla sie");
        }
        Err(e) => panic!("cl: {}", e),
    }
}

/// Panel projektow i czyszczenia w C# -> indeks-panel.exe.
fn build_panel_engine() {
    println!("cargo:rerun-if-changed=csharp/Panel.cs");
    println!("cargo:rerun-if-changed=csharp/PanelDane.cs");
    println!("cargo:rerun-if-changed=csharp/PanelRysuj.cs");
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let csc = std::path::Path::new(&windir).join(r"Microsoft.NET\Framework64\v4.0.30319\csc.exe");
    let Some(target) = target_dir() else { return };
    let out = std::process::Command::new(&csc)
        .args(["/nologo", "/target:winexe", "/optimize+", "/platform:x64", "/r:System.Windows.Forms.dll", "/r:System.Drawing.dll"])
        .arg(format!("/out:{}", target.join("indeks-panel.exe").display()))
        .arg(std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("csharp").join("Panel.cs"))
        .arg(std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("csharp").join("PanelDane.cs"))
        .arg(std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("csharp").join("PanelRysuj.cs"))
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("error")) {
                println!("cargo:warning=csc: {}", line);
            }
            panic!("kompilacja csharp/Panel.cs nie powiodla sie");
        }
        Err(e) => panic!("csc: {}", e),
    }
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

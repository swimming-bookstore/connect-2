fn main() {
    let public = std::path::Path::new("../ui/public");
    println!("cargo:rerun-if-changed=../ui/public/index.html");
    println!("cargo:rerun-if-changed=../ui/public/style.css");
    println!("cargo:rerun-if-changed=../ui/public/pkg");
    if !public.join("index.html").is_file() {
        println!("cargo:warning=ui/public/index.html missing; build connect2 --features web first");
    }
    tauri_build::build()
}

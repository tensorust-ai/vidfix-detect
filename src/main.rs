use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if files.is_empty() {
        eprintln!("usage: vidfix-detect [--json] <file.mp4|file.mov> ...");
        exit(2);
    }
    let mut code = 0;
    for f in files {
        match vidfix_detect::diagnose_file(f) {
            Ok(d) if json => println!("{}", d.json()),
            Ok(d) => {
                println!("{}: {:?} ({})", f, d.verdict, d.device);
                println!("  {}", d.message());
                if let Some(u) = d.repair_url() {
                    println!("  Free preview of the rebuilt file: {}", u);
                }
                if d.verdict != vidfix_detect::Verdict::Healthy {
                    code = 1;
                }
            }
            Err(e) => {
                eprintln!("{}: {}", f, e);
                code = 2;
            }
        }
    }
    exit(code);
}

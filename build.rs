fn main() {
    // icona e informazioni dell'eseguibile su Windows
    #[cfg(windows)]
    {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/popmsg.ico");
        r.set("FileDescription", "POPMSG");
        r.set("ProductName", "POPMSG");
        let _ = r.compile();
    }
}

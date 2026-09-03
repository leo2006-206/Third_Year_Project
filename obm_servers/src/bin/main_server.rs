use smol::{
    io,
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    prelude::*,
};

async fn handle_client(mut stream: TcpStream) -> io::Result<()> {
    let mut buf = vec![0u8; 1024];

    // Read data from the client
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        println!("Server received: {}", String::from_utf8_lossy(&buf[..n]));
        println!("{:?}", buf);
    }
    Ok(())
}

fn main() -> io::Result<()> {
    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = "0.0.0.0:7000";
        let listener = TcpListener::bind(port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream).await {
                    eprintln!("Error handling client: {}", e);
                }
            })
            .detach(); // Detach allows the task to run independently in the background
        }
        Ok(())
    })
}

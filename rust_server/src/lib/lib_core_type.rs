use smol::io::{AsyncRead, AsyncWrite};

pub trait Reader: AsyncRead + Unpin {}

impl<T> Reader for T where T: AsyncRead + Unpin {}

pub trait Writer: AsyncWrite + Unpin {}

impl<T> Writer for T where T: AsyncWrite + Unpin {}

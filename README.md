# Smallchat

This is a rewrite of https://github.com/antirez/smallchat in Rust.
We are still under 200Loc, while providing some additional features.
The only library used is `mio`, which is necessary to provide a cross platform
abstraction of `epoll`.
 
Additional features:
- Memory safe (eheheh)
- Buffering of input and output

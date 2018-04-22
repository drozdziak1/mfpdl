# MFPDL
`mfpdl` is a small CLI crawler that visits Datassette's [Music for
programming](http://musicforprogramming.net) website and downloads all the music it
can find. Presently it uses `wget -N` so that it only downloads the sets not
present locally.

## Planned features
* Move on to asynchronous downloads for quicker operation
* An option for picking the directory to download to
* A dry-run flag for only dumping the file URLs to stdout

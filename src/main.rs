use clap::{App, Arg, ArgMatches};
use failure::format_err;
use futures::{lock::Mutex, prelude::*};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use lazy_static::lazy_static;
use reqwest::Response;
use scraper::{Html, Selector};
use tokio::{fs::OpenOptions, prelude::*, sync::Semaphore, task};

use std::{io, iter::Iterator, path::PathBuf, sync::Arc, fs};

const MFP_URL: &'static str = "https://www.musicforprogramming.net";
const DEFAULT_N_JOBS: &'static str = "8";

// HTML element selectors for the scraper lib, reused across downloads
lazy_static! {
    static ref MFP_FILE_SELECTOR: Selector = Selector::parse("div .pad a[href$=mp3]")
        .map_err(|e| format_err!("Could not parse the file selector: {:?}", e))
        .unwrap();
    static ref MFP_EP_SELECTOR: Selector = Selector::parse("#episodes a")
        .map_err(|e| format_err!("Could not parse the episode selector: {:?}", e))
        .unwrap();
}

type ErrBox = Box<dyn std::error::Error>;

/// Downloads a `reqwest::Response` to the specified location while respecting a job count quota
/// guarded by `sema`. `bars` must contain as many bar entries as there are permits in the
/// semaphore.
async fn download_with_sema(
    resp: Response,
    sema: Arc<Semaphore>,
    bars: Arc<Vec<Mutex<ProgressBar>>>,
    path: PathBuf,
) -> Result<(), ErrBox> {
    // Wait for a free progress bar
    let _permit = sema.acquire().await;
    let pb = bars
        .iter()
        .filter_map(|mutex| mutex.try_lock())
        .nth(0)
        .ok_or_else(|| format_err!("Could not acquire a lock for a progress bar despite permit"))?;

    // Find out when the progress bar should end
    let len = resp
        .content_length()
        .ok_or_else(|| format_err!("Could not get Content-Length for {:?}", path))?;

    // Prepare the progress bar
    pb.set_length(len as u64);
    pb.set_position(0);
    pb.set_message(
        path.file_name()
            .ok_or_else(|| format_err!("Could not get file name from path for {:?}", path))?
            .to_str()
            .unwrap(),
    );

    let mut file = match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .await
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            pb.println(format!("File {:?} already exists, skipping...", path));
            pb.set_message("Idle");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    // Stream the response to a file
    let mut stream = resp.bytes_stream().err_into::<ErrBox>();

    while let Some(res) = stream.next().await {
        let bytes = res?;
        file.write_all(&bytes).await?;
        pb.inc(bytes.len() as u64);
    }

    Ok(())
}

/// Retrieve a file URL for the specified musicforprogramming.net episode URL
async fn scrape_episode_file_url(url: &str) -> Result<String, ErrBox> {
    let resp = reqwest::get(url).await?;
    let fragment = Html::parse_document(&resp.text().await?);

    let file_url = fragment
        .select(&*MFP_FILE_SELECTOR)
        .nth(0)
        .ok_or_else(|| format_err!("Could not find file URL for {}", url))?
        .value()
        .attr("href")
        .ok_or_else(|| format_err!("Could not find href for file URL element in {}", url))?;

    Ok(file_url.to_owned())
}

fn cli_setup<'a>() -> ArgMatches<'a> {
    App::new(env!("CARGO_PKG_NAME"))
        .arg(
            Arg::with_name("latest")
                .long("latest")
                .short("l")
                .help("Only get the latest episode")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("jobs")
                .long("jobs")
                .short("j")
                .help("How many episodes should be downloaded concurrently")
                .takes_value(true)
                .required(false)
                .validator(|val| {
                    let v = val
                        .parse::<usize>()
                        .map_err(|e| format!("Could not parse as number: {}", e.to_string()))?;
                    if v == 0 {
                        return Err("Job count cannot be 0".to_owned());
                    }
                    Ok(())
                })
                .default_value(DEFAULT_N_JOBS),
        )
        .arg(
            Arg::with_name("outdir")
                .long("dir")
                .short("d")
                .help("Put the files in <outdir>. Will be created if doesn't exist")
                .takes_value(true)
                .required(false)
                .validator(|path| {
                    let p = path
                        .parse::<PathBuf>()
                        .map_err(|e| format!("Could not parse as a path: {}", e.to_string()))?;
                    if p.exists() && p.is_file() {
                        return Err("Existing path must not be a file".to_owned());
                    }
                    Ok(())
                })
                .default_value("."),
        )
        .get_matches()
}

#[tokio::main]
async fn main() -> Result<(), ErrBox> {
    let matches = cli_setup();
    // Setup the MultiProgress bar
    let mpb = MultiProgress::new();

    let n_jobs = matches.value_of("jobs").unwrap().parse()?;

    let outdir = matches.value_of("outdir").unwrap().parse::<PathBuf>()?;
    fs::create_dir_all(&outdir)?;

    // Setup the shared bars lock
    let bars = {
        let v = (0..n_jobs)
            .map(|_n| {
                let pb = ProgressBar::new(0);
                pb.set_style(ProgressStyle::default_bar().template("{msg} {bar} {pos}/{len}"));
                mpb.add(pb.clone());
                Mutex::new(pb)
            })
            .collect::<Vec<_>>();
        Arc::new(v)
    };

    // Setup a semaphore for tracking available bars
    let sema = Arc::new(Semaphore::new(n_jobs));

    // Obtain the main page
    let resp = reqwest::get(MFP_URL).await?;
    if !resp.status().is_success() {
        panic!("Request failed for {}", MFP_URL);
    }

    // Scrape latest episode file URL
    let latest_url = scrape_episode_file_url(MFP_URL).await?;

    let latest_fname = latest_url.split("/").last().unwrap();
    let latest_resp = reqwest::get(&latest_url).await?;

    let latest_fut = download_with_sema(
        latest_resp,
        sema.clone(),
        bars.clone(),
        outdir.join(latest_fname),
    );

    // Scrape the rest of the espiode file URLs
    let body = resp.text().await?;
    let fragment = Html::parse_document(&body);

    let dl_futs = fragment.select(&*MFP_EP_SELECTOR).map(|episode| {
        let bars4fut = bars.clone();
        let sema4fut = sema.clone();
        let outdir4fut = outdir.clone();
        async move {
            let subpage = episode.value().attr("href").unwrap();
            let ep_url = &format!("{}/{}", MFP_URL, subpage);

            let file_url = scrape_episode_file_url(ep_url).await?;
            let fname = file_url.split("/").last().unwrap();

            let file_resp = reqwest::get(&file_url).await?;

            download_with_sema(file_resp, sema4fut, bars4fut, outdir4fut.join(fname.to_owned())).await?;

            Result::<(), ErrBox>::Ok(())
        }
    });

    let downloads_joined = future::try_join_all(dl_futs).err_into::<ErrBox>();

    let bar_join_fut = async move {
        task::spawn_blocking(move || mpb.join_and_clear())
            .err_into::<ErrBox>()
            .await??;
        Result::<(), ErrBox>::Ok(())
    };

    let cleanup_after_download_fut = async move {
        if matches.is_present("latest") {
            latest_fut.await?;
        } else {
            future::try_join(latest_fut, downloads_joined).await?;
        }

        // Required to unblock the MultiProgress bar
        for mutex in bars.iter() {
            mutex.lock().await.finish();
        }

        Result::<(), ErrBox>::Ok(())
    };

    future::try_join(cleanup_after_download_fut, bar_join_fut).await?;

    Ok(())
}

use failure::format_err;
use futures::{lock::Mutex, prelude::*};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Response;
use scraper::{Html, Selector};
use tokio::{sync::Semaphore, task};

use std::{iter::Iterator, path::PathBuf, sync::Arc};

const MFP_URL: &'static str = "https://www.musicforprogramming.net";
const MFP_FILE_SELECTOR: &'static str = "div .pad a[href$=mp3]";
const MFP_EP_SELECTOR: &'static str = "#episodes a";
const N_JOBS: usize = 2;

type ErrBox = Box<dyn std::error::Error>;

#[derive(Clone, Debug)]
struct BarEntry {
    pub is_free: bool,
    pub pb: ProgressBar,
}

/// Downloads a `reqwest::Response` to the specified location while respecting a job count quota
/// guarded by `sema`. `bars` must contain as many bar entries as there are permits in the
/// semaphore.
async fn download_with_sema(
    resp: Response,
    sema: Arc<Semaphore>,
    bars: Arc<Mutex<Vec<BarEntry>>>,
    fname: PathBuf,
) -> Result<(), ErrBox> {
    // Wait for a free progress bar
    let permit = sema.acquire().await;
    let (bar_idx, bar_entry) = {
        let mut lock = bars.lock().await;
        let idx = lock
            .iter_mut()
            .enumerate()
            .filter(|(_, b)| b.is_free)
            .nth(0)
            .unwrap() // Guaranteed Some() by the semaphore
            .0;
        lock[idx].is_free = false;
        (idx, lock[idx].clone())
    };

    // Find out when the progress bar should end
    let len = resp
        .content_length()
        .ok_or_else(|| format_err!("Could not get Content-Length for {:?}", fname))?;

    // Prepare the progress bar
    bar_entry.pb.set_length(len as u64);
    bar_entry.pb.set_position(0);
    bar_entry.pb.set_message(
        fname
            .file_name()
            .ok_or_else(|| format_err!("Could not get file name from path for {:?}", fname))?
            .to_str()
            .unwrap(),
    );

    // Stream the response to a file
    resp.bytes_stream()
        .err_into::<ErrBox>()
        .try_for_each(move |bytes| {
            let pb4fut = bar_entry.pb.clone();
            async move {
                pb4fut.inc(bytes.len() as u64);
                Ok(())
            }
        })
        .await?;

    // Free the bar after use
    bars.lock().await[bar_idx].is_free = true;
    drop(permit);
    Ok(())
}

/// Retrieve a file URL for the specified musicforprogramming.net episode URL
async fn scrape_episode_file_url(url: &str) -> Result<String, ErrBox> {
    let file_selec = Selector::parse(MFP_FILE_SELECTOR)
        .map_err(|e| format_err!("Could not parse the file selector: {:?}", e))?;
    let resp = reqwest::get(url).await?;
    let fragment = Html::parse_document(&resp.text().await?);

    let file_url = fragment
        .select(&file_selec)
        .nth(0)
        .ok_or_else(|| format_err!("Could not find file URL for {}", url))?
        .value()
        .attr("href")
        .ok_or_else(|| format_err!("Could not find href for file URL element in {}", url))?;

    Ok(file_url.to_owned())
}

#[tokio::main]
async fn main() -> Result<(), ErrBox> {
    // Setup the MultiProgress bar
    let mpb = MultiProgress::new();

    // Setup the shared bars lock
    let bars = {
        let v = (0..N_JOBS)
            .map(|_n| {
                let pb = ProgressBar::new(0);
                pb.set_style(ProgressStyle::default_bar().template("{msg} {bar} {pos}/{len}"));
                mpb.add(pb.clone());
                BarEntry { is_free: true, pb }
            })
            .collect::<Vec<_>>();
        Arc::new(Mutex::new(v))
    };

    // Setup a semaphore for tracking available bars
    let sema = Arc::new(Semaphore::new(N_JOBS));

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
        latest_fname.to_owned().into(),
    );

    // Scrape the rest of the espiode file URLs
    let body = resp.text().await?;
    let fragment = Html::parse_document(&body);

    let ep_selec = Selector::parse(MFP_EP_SELECTOR).expect("Couldn't parse the episode selector");

    let dl_futs = fragment.select(&ep_selec).map(|episode| {
        let bars4fut = bars.clone();
        let sema4fut = sema.clone();
        async move {
            let subpage = episode.value().attr("href").unwrap();
            let ep_url = &format!("{}/{}", MFP_URL, subpage);

            let file_url = scrape_episode_file_url(ep_url).await?;
            let fname = file_url.split("/").last().unwrap();

            let file_resp = reqwest::get(&file_url).await?;

            download_with_sema(file_resp, sema4fut, bars4fut, fname.to_owned().into()).await?;

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

    future::try_join3(downloads_joined, latest_fut, bar_join_fut).await?;

    Ok(())
}

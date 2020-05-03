use failure::format_err;
use futures::prelude::*;
use indicatif::{MultiProgress, ProgressBar};
use reqwest::Response;
use scraper::{Html, Selector};
use tokio::{ task};

use std::iter::Iterator;

const MFP_URL: &'static str = "https://www.musicforprogramming.net";

type ErrBox = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), ErrBox> {
    let resp = reqwest::get(MFP_URL).await?;
    if !resp.status().is_success() {
        panic!("Request failed for {}", MFP_URL);
    }

    let body = resp.text().await?;

    let fragment = Html::parse_document(&body);

    let file_selec =
        Selector::parse("div .pad a[href$=mp3]").expect("Couldn't parse the file link selector");

    let latest_url = fragment
        .select(&file_selec)
        .nth(0)
        .expect("Couldn't find latest URL")
        .value()
        .attr("href")
        .expect(&format!("Failed to get href for latest"));

    let latest_fname = latest_url.split("/").last().unwrap();
    let latest_resp = reqwest::get(latest_url).await?;
    let latest_len = latest_resp
        .content_length()
        .ok_or_else(|| format_err!("Could not get Content-Length on latest episode"))?;
    let latest_pb = ProgressBar::new(latest_len);

    // Setup the MultiProgress bar
    let mpb = MultiProgress::new();
    mpb.add(latest_pb.clone());

    let latest_fut = latest_resp
        .bytes_stream()
        .err_into::<ErrBox>()
        .try_for_each(|bytes| {
            let pb4fut = latest_pb.clone();
            async move {
                pb4fut.inc(bytes.len() as u64);
                Ok(())
            }
        });

    // Scrape links and prepare progress bars
    let ep_selec = Selector::parse("#episodes a").expect("Couldn't parse the episode selector");

    let scrape_futs = fragment
        .select(&ep_selec)
        .enumerate()
        .map(|(idx, episode)| async move {
            let file_selec = Selector::parse("div .pad a[href$=mp3]")
                .expect("Couldn't parse the file link selector");

            let subpage = episode.value().attr("href").unwrap();
            let ep_url = &format!("{}/{}", MFP_URL, subpage);
            let subpage_resp = reqwest::get(ep_url).await?;

            let body = subpage_resp.text().await?;

            let subpage_fragment = Html::parse_document(&body);

            let file_url = subpage_fragment
                .select(&(file_selec.clone()))
                .nth(0)
                .expect(&format!("Couldn't find file URL for {}", idx + 1))
                .value()
                .attr("href")
                .expect(&format!("Failed to get href for {}", idx + 1));

            let fname = file_url.split("/").last().unwrap();
            println!("Scraping {}... ", fname);

            let file_resp = reqwest::get(file_url).await?;

            let file_len = file_resp
                .content_length()
                .ok_or_else(|| format_err!("Could not get Content-Length for {}", fname))?;

            let pb = ProgressBar::new(file_len);

            Result::<(Response, String, ProgressBar), ErrBox>::Ok((file_resp, fname.to_owned(), pb))
        });

    let mut scraped = future::try_join_all(scrape_futs).await?;

    println!("Foud {} episodes, downloading", scraped.len() + 1);

    let dl_futs = scraped.drain(..).map(|(resp, fname, pb)| {
        let resp4fut = resp;
        mpb.add(pb.clone());
        async move {
            resp4fut
                .bytes_stream()
                .err_into::<ErrBox>()
                .try_for_each(move |bytes| {
                    let pb4fut = pb.clone();
                    async move {
                        pb4fut.inc(bytes.len() as u64);
                        Ok(())
                    }
                })
                .await?;
            Result::<(), ErrBox>::Ok(())
        }
    });

    let downloads_joined = future::try_join_all(dl_futs).err_into::<ErrBox>();

    let bar_join_fut = async move {
        task::spawn_blocking(move || mpb.join_and_clear())
            .err_into::<ErrBox>()
            .await??;
        Ok(())
    };

    future::try_join3(downloads_joined, latest_fut, bar_join_fut).await?;

    Ok(())
}

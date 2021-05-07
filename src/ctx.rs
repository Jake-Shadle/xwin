use anyhow::Error;
use std::path::{Path, PathBuf};

pub struct Ctx {
    work_dir: PathBuf,
    tempdir: Option<tempfile::TempDir>,
    client: reqwest::Client,
}

impl Ctx {
    pub fn with_temp() -> Result<Self, Error> {
        let td = tempfile::TempDir::new()?;
        let client = reqwest::ClientBuilder::new().build()?;

        Ok(Self {
            work_dir: td.path().to_owned(),
            tempdir: Some(td),
            client,
        })
    }

    pub fn with_dir(work_dir: PathBuf) -> Result<Self, Error> {
        let client = reqwest::ClientBuilder::new().build()?;

        Ok(Self {
            work_dir,
            tempdir: None,
            client,
        })
    }

    pub async fn get<P: AsRef<Path>>(&self, url: String, path: &P) -> Result<bytes::Bytes, Error> {
        self.get_and_validate(url, path, |_| true).await
    }

    pub async fn get_and_validate<V>(
        &self,
        url: String,
        path: &impl AsRef<Path>,
        validate: V,
    ) -> Result<bytes::Bytes, Error>
    where
        V: Fn(&[u8]) -> bool + Send + 'static,
    {
        let cache_path = self.work_dir.join(path);

        if cache_path.exists() {
            if let Ok(contents) = std::fs::read(&cache_path) {
                if validate(&contents) {
                    return Ok(contents.into());
                }
            }
        }

        let res = self.client.get(&url).send().await?.error_for_status()?;

        let body = res.bytes().await?;

        let body = tokio::task::spawn_blocking(move || {
            if !validate(&body) {
                anyhow::bail!("failed to validate response body");
            }

            std::fs::write(cache_path, &body)?;
            Ok(body)
        })
        .await??;

        Ok(body)
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        if let Some(td) = self.tempdir.take() {
            let _ = td.close();
        }
    }
}

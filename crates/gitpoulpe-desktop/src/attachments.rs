use gitpoulpe_hosting::{Attachment, AttachmentKind, Preview, fetch_preview};
use rune::{Image, Tx};

use crate::engine::Event;
use crate::state::State;

#[derive(Clone, Debug)]
pub enum Media {
    Loading,
    Decoding(Image),
    Ready(Image, [u32; 2]),
    Failed(String),
}

impl State {
    fn pull_attachments(&self) -> impl Iterator<Item = &Attachment> {
        let list = &self.pulls.list;
        let body = list.detail.iter().flat_map(|d| d.attachments.iter());
        let issue = list
            .comments
            .issue
            .iter()
            .flat_map(|c| c.attachments.iter());
        let review = list
            .comments
            .review
            .iter()
            .flat_map(|c| c.attachments.iter());
        body.chain(issue).chain(review)
    }

    pub fn fetch_pull_media(&mut self) {
        let Some(number) = self.pulls.list.selected else {
            return;
        };
        let Some((connection, _, _)) = self.pull_list_target() else {
            return;
        };
        let images = self
            .pull_attachments()
            .filter(|a| a.kind == AttachmentKind::Image)
            .filter_map(|a| Some((a.link.as_str(), a.preview.as_ref()?)));
        let versions = self
            .pulls
            .list
            .detail
            .iter()
            .flat_map(|d| d.files.iter())
            .filter_map(|f| f.media.as_ref())
            .flat_map(|m| [m.before.as_ref(), m.after.as_ref()])
            .flatten()
            .map(|p| (p.key(), p));
        let mut wanted: Vec<(String, Preview)> = Vec::new();
        for (key, preview) in images.chain(versions) {
            if !self.pulls.list.media.contains_key(key) && !wanted.iter().any(|(k, _)| k == key) {
                wanted.push((key.to_owned(), preview.clone()));
            }
        }
        for (link, preview) in wanted {
            self.pulls.list.media.insert(link.clone(), Media::Loading);
            let secrets = self.hosting.secrets.clone();
            let slots = self.hosting.media_slots.clone();
            let connection = connection.clone();
            let sender = self.event_tx();
            let waker = self.waker_slot();
            self.hosting.runtime.spawn(async move {
                let _slot = slots.acquire_owned().await;
                let result = fetch_preview(secrets.as_ref(), &connection, &preview)
                    .await
                    .map_err(|e| e.to_string());
                let _ = sender.send(Event::PullMediaLoaded {
                    number,
                    link,
                    result,
                });
                if let Some(wake) = waker.lock().unwrap().as_ref() {
                    wake();
                }
            });
        }
    }

    pub fn apply_pull_media(
        &mut self,
        number: u64,
        link: String,
        result: Result<Vec<u8>, String>,
    ) -> bool {
        let list = &mut self.pulls.list;
        if !list.open || list.selected != Some(number) {
            return false;
        }
        let Some(entry) = list.media.get_mut(&link) else {
            return false;
        };
        *entry = match (result, &self.images) {
            (Ok(bytes), Some(cache)) => Media::Decoding(cache.load(bytes)),
            (Ok(_), None) => Media::Failed("décodeur d'images indisponible".to_owned()),
            (Err(why), _) => Media::Failed(why),
        };
        self.settle_pull_media();
        self.pulls.list.version += 1;
        true
    }

    pub fn settle_pull_media(&mut self) -> bool {
        let mut changed = false;
        for media in self.pulls.list.media.values_mut() {
            if let Media::Decoding(image) = media
                && image.is_ready()
            {
                *media = match image.size() {
                    Some(size) => Media::Ready(image.clone(), size),
                    None => Media::Failed(
                        "format d'image non pris en charge ou fichier abîmé".to_owned(),
                    ),
                };
                changed = true;
            }
        }
        if changed {
            self.pulls.list.version += 1;
        }
        changed
    }

    pub fn open_link(&mut self, tx: &mut Tx, url: &str) {
        if let Err(why) = tx.open_url(url) {
            self.pulls.list.notice = Some(format!("Impossible d'ouvrir le lien : {why}"));
            self.pulls.list.version += 1;
        }
    }
}

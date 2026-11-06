//! Cache only metadata belonging to a currently discoverable native volume.
use super::*;

impl Presenter {
    pub(super) fn volume_details_updated(
        &mut self,
        id: String,
        root: PathBuf,
        details: VolumeDetails,
    ) {
        if self
            .volumes
            .iter()
            .any(|volume| volume.id == id && volume.root == root)
        {
            if let Some(old) = self.volume_details.insert(id, (root, details)) {
                self.retired.push(Box::new(old));
            }
        } else {
            self.retired.push(Box::new((id, root, details)));
        }
    }

    pub(super) fn retain_volume_details(&mut self, volumes: &[Volume]) {
        let mut previous = mem::take(&mut self.volume_details);
        for volume in volumes {
            if let Some((root, details)) = previous.remove(&volume.id) {
                if root == volume.root {
                    self.volume_details
                        .insert(volume.id.clone(), (root, details));
                } else {
                    self.retired.push(Box::new((root, details)));
                }
            }
        }
        self.retired.push(Box::new(previous));
    }
}

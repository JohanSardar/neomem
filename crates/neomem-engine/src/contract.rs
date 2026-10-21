use std::fmt::Write as _;

use neomem_core::{Choice, ExternalLink, ExternalLinkKind, ExternalRef, JudgeRole, MemoryId, Purpose, Tag};
use neomem_proto::{FailureKind, Response};

use crate::{Engine, reply::failed};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ContractError {
    #[error(transparent)]
    State(#[from] neomem_state::StateError),
    #[error(transparent)]
    Model(#[from] neomem_core::ModelError),
    #[error(transparent)]
    Tag(#[from] neomem_core::InvalidTag),
}

impl Engine {
    pub(crate) fn tags(&self, new: Option<&str>) -> Response {
        let _writing = self.lock_writer();
        let result = new.map_or_else(
            || self.state.tags().map_err(ContractError::from),
            |text| self.canonical_tag(text).map(|tag| vec![tag]),
        );
        match result {
            Ok(tags) => Response::Tags(tags),
            Err(ContractError::Tag(error)) => failed(FailureKind::BadRequest, error.to_string()),
            Err(error) => failed(FailureKind::Unavailable, error.to_string()),
        }
    }

    pub(crate) fn canonical_tag(&self, text: &str) -> Result<Tag, ContractError> {
        let candidate = Tag::new(text)?;
        let tags = self.state.tags()?;
        if tags.contains(&candidate) {
            return Ok(candidate);
        }
        let embedder = self.models.embedder()?;
        let mut texts: Vec<&str> = vec![candidate.as_str()];
        texts.extend(tags.iter().map(Tag::as_str));
        let vectors = embedder.embed(&texts, Purpose::Document)?;
        if vectors.len() != texts.len() {
            return Err(
                neomem_core::ModelError::InvalidOutput("tag embedding count mismatch".to_owned()).into()
            );
        }
        let mut closest: Vec<_> = tags.iter().zip(vectors.iter().skip(1)).collect();
        closest.sort_by(|(_, left), (_, right)| vectors[0].cosine(right).total_cmp(&vectors[0].cosine(left)));
        closest.truncate(8);
        let mut context = format!("Candidate tag: {candidate}\nExisting tags:\n");
        let mut labels = Vec::new();
        for (index, (tag, _)) in closest.iter().enumerate() {
            let number = index + 1;
            let _ = writeln!(context, "{number}. {tag}");
            labels.push((format!("same_{number}"), format!("same meaning as existing tag {number}")));
        }
        labels.push(("new".to_owned(), "a genuinely new meaning".to_owned()));
        let options: Vec<_> = labels.iter().map(|(label, meaning)| Choice { label, meaning }).collect();
        let decision = self.models.judge(JudgeRole::Judge)?.decide(
            &context,
            "Does this tag have the same meaning as an existing tag, or is it new?",
            &options,
        )?;
        if decision.probabilities.len() != options.len() {
            return Err(
                neomem_core::ModelError::InvalidOutput("tag judge choice count mismatch".to_owned()).into()
            );
        }
        if let Some((tag, _)) = closest.get(decision.choice()) {
            return Ok((*tag).clone());
        }
        self.state.register_tag(&candidate)?;
        Ok(candidate)
    }

    pub(crate) fn link_external(
        &self,
        external: ExternalRef,
        to: MemoryId,
        kind: ExternalLinkKind,
    ) -> Response {
        let _writing = self.lock_writer();
        let snapshot = self.index.snapshot();
        let target = match to {
            MemoryId::Entity(id) if snapshot.entity(id).is_some() => MemoryId::Entity(snapshot.survivor(id)),
            MemoryId::Fact(id) if snapshot.fact(id).is_some() => to,
            _ => return failed(FailureKind::NotFound, format!("unknown external link target {to}")),
        };
        let link = ExternalLink { external, to: target, kind, recorded_at: self.clock.now() };
        if let Err(error) = self.state.register_external_link(&link) {
            return failed(FailureKind::Internal, error.to_string());
        }
        match self.index.register_external_links(std::slice::from_ref(&link)) {
            Ok(()) => Response::Linked(link),
            Err(error) => failed(FailureKind::Internal, error.to_string()),
        }
    }
}

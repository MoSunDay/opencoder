use super::super::{store::check_revision, Store};
use crate::{
    domain::{validate_aspect, AspectPatch, GraphAspect},
    error::AppError,
};
use uuid::Uuid;

impl Store {
    pub(crate) async fn graph_aspects(
        &self,
        env: i64,
        deleted: bool,
    ) -> Result<Vec<GraphAspect>, AppError> {
        Ok(self
            .list::<GraphAspect>("graph_aspects", env)
            .await?
            .into_iter()
            .filter(|item| deleted || !item.is_deleted)
            .collect())
    }

    pub(crate) async fn graph_aspect(&self, env: i64, id: Uuid) -> Result<GraphAspect, AppError> {
        let item: GraphAspect = self.get("graph_aspects", env, id).await?;
        if item.is_deleted {
            Err(AppError::NotFound)
        } else {
            Ok(item)
        }
    }

    async fn validate_aspect_refs(&self, env: i64, patch: &AspectPatch) -> Result<(), AppError> {
        let types = self.entity_types(env, false).await?;
        let relations = self.relationship_types(env, false).await?;
        if patch
            .entity_type_ids
            .iter()
            .any(|id| !types.iter().any(|item| item.id == *id))
            || patch
                .relationship_type_ids
                .iter()
                .any(|id| !relations.iter().any(|item| item.id == *id))
        {
            return Err(AppError::invalid(
                "aspect references an inactive or foreign definition",
            ));
        }
        for id in &patch.defaults.default_center_ids {
            let entity = self.entity(env, *id).await?;
            if entity.is_deleted || !patch.entity_type_ids.contains(&entity.entity_type_id) {
                return Err(AppError::invalid(
                    "aspect center must be an active entity in the selected types",
                ));
            }
        }
        Ok(())
    }

    pub(crate) async fn create_graph_aspect(
        &self,
        env: i64,
        input: crate::database::model::inputs::AspectCreate<'_>,
        actor: &str,
    ) -> Result<GraphAspect, AppError> {
        let crate::database::model::inputs::AspectCreate {
            key,
            title,
            description,
            types,
            relations,
            defaults,
        } = input;
        let patch = validate_aspect(key, title, description, types, relations, defaults)?;
        self.validate_aspect_refs(env, &patch).await?;
        if self
            .graph_aspects(env, true)
            .await?
            .iter()
            .any(|item| item.aspect_key == patch.aspect_key)
        {
            return Err(AppError::Conflict("aspect key already exists".into()));
        }
        let item = GraphAspect {
            id: Uuid::new_v4(),
            env_num: env,
            aspect_key: patch.aspect_key,
            name: patch.name,
            description: patch.description,
            entity_type_ids: patch.entity_type_ids,
            relationship_type_ids: patch.relationship_type_ids,
            defaults: patch.defaults,
            revision: 1,
            is_deleted: false,
        };
        self.put("graph_aspects", env, item.id, &item).await?;
        self.audit(env, actor, "aspect.create", item.id, 1).await?;
        Ok(item)
    }

    pub(crate) async fn update_graph_aspect(
        &self,
        env: i64,
        id: Uuid,
        input: crate::database::model::inputs::AspectUpdate<'_>,
        actor: &str,
    ) -> Result<GraphAspect, AppError> {
        let crate::database::model::inputs::AspectUpdate {
            title,
            description,
            types,
            relations,
            defaults,
            deleted,
            expected,
        } = input;
        let mut item: GraphAspect = self.get("graph_aspects", env, id).await?;
        check_revision(item.revision, expected)?;
        let patch = validate_aspect(
            &item.aspect_key,
            title,
            description,
            types,
            relations,
            defaults,
        )?;
        if !deleted {
            self.validate_aspect_refs(env, &patch).await?;
        }
        item.name = patch.name;
        item.description = patch.description;
        item.entity_type_ids = patch.entity_type_ids;
        item.relationship_type_ids = patch.relationship_type_ids;
        item.defaults = patch.defaults;
        item.is_deleted = deleted;
        item.revision += 1;
        self.put("graph_aspects", env, id, &item).await?;
        self.audit(env, actor, "aspect.update", id, item.revision)
            .await?;
        Ok(item)
    }

    pub(crate) async fn soft_delete_graph_aspect(
        &self,
        env: i64,
        id: Uuid,
        expected: i64,
        actor: &str,
    ) -> Result<GraphAspect, AppError> {
        let item = self.graph_aspect(env, id).await?;
        self.update_graph_aspect(
            env,
            id,
            crate::database::model::inputs::AspectUpdate {
                title: &item.name,
                description: &item.description,
                types: item.entity_type_ids,
                relations: item.relationship_type_ids,
                defaults: item.defaults,
                deleted: true,
                expected,
            },
            actor,
        )
        .await
    }
}

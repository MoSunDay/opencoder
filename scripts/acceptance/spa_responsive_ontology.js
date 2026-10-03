const base = '/api/ontology';
const env = `${base}/envs/debug`;
const metadata = { env_num: 1, revision: 1, is_deleted: false, description: '验收示例数据' };
const types = [
  { ...metadata, id: '00000000-0000-4000-8000-000000000001', type_key: 'directory', name: '目录', is_system: true },
  { ...metadata, id: 'service', type_key: 'service', name: '服务', is_system: false },
];
const entities = [
  { ...metadata, id: 'entry', entity_type_id: 'service', name: '入口服务' },
  { ...metadata, id: 'downstream', entity_type_id: 'service', name: '下游服务' },
];
const relationshipTypes = [{ ...metadata, id: 'calls', type_key: 'calls', name: '调用',
  source_entity_type_id: 'service', target_entity_type_ids: ['service'], is_directory_membership: false, is_system: false }];
const relationships = [{ ...metadata, id: 'call', relationship_type_id: 'calls',
  source_entity_id: 'entry', target_entity_id: 'downstream', is_pinned: false }];
const attributes = [{ ...metadata, id: 'source', entity_type_id: 'service', attribute_key: 'source',
  name: '来源', kind: 'text', attribute_role: 'source', storage_mode: 'markdown', required: false }];

const ONTOLOGY_FIXTURES = {
  [`${base}/nfs`]: { root: '/fixture/ontology', status: { running: true, host: '127.0.0.1', port: 2052, read_only: true, export_root: '/fixture/ontology' } },
  [`${base}/environments`]: { items: [{ ...metadata, env_key: 'debug', name: '调试环境', initialization_status: 'ready' }] },
  [`${env}/entity-types`]: { items: types },
  [`${env}/entities`]: { items: entities },
  [`${env}/directories/tree`]: { root_id: 'directory-root', items: [{ ...metadata, id: 'directory-root', name: '根目录' }] },
  [`${env}/relationship-types`]: { items: relationshipTypes },
  [`${env}/relationships`]: { items: relationships },
  [`${env}/graph-aspects`]: { items: [{ ...metadata, id: 'aspect', aspect_key: 'calls', name: '服务调用',
    entity_type_ids: ['service'], relationship_type_ids: ['calls'], default_center_ids: ['entry'], default_upstream_depth: 3, default_downstream_depth: 3 }] },
  [`${env}/graph`]: { nodes: entities, edges: relationships, available_relationship_type_ids: ['calls'] },
};
for (const type of types) {
  ONTOLOGY_FIXTURES[`${env}/entity-types/${type.id}/attributes`] = { items: type.id === 'service' ? attributes : [] };
  ONTOLOGY_FIXTURES[`${env}/entity-types/${type.id}/actions`] = { items: [] };
}
for (const entity of entities) {
  ONTOLOGY_FIXTURES[`${env}/entities/${entity.id}`] = { item: entity, attribute_definitions: attributes,
    structured_attributes: [], text_attributes: [], actions: [], needs_completion: false };
}
module.exports = { ONTOLOGY_FIXTURES };

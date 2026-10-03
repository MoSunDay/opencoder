"""Verify durable Ontology references and the same NFS owner across Server switches."""
import uuid


def seed(env):
    prefix = '/api/ontology/envs/debug'
    kind = env.api(prefix+'/entity-types','POST',{'key':'release_proof','name':'发布证明'})['item']['id']
    entity = env.api(prefix+'/entities','POST',{'request_id':str(uuid.uuid4()),'entity_type_id':kind,
        'name':'Ontology 连续性','source':'first source','ext':'first extension'})['item']['id']
    detail = env.api(prefix+'/entities/'+entity)
    source = next(item for item in detail['text_attributes'] if item['definition']['attribute_role']=='source')
    aspect = env.api(prefix+'/graph-aspects','POST',{'key':'release_proof','name':'发布切面',
        'entity_type_ids':[kind],'relationship_type_ids':[],'default_center_ids':[entity]})['item']['id']
    proof = {'entity':entity,'source':source['definition']['id'],'aspect':aspect,
        'path':source['current']['content_path'],'content':'first source','revision':1}
    verify(env, proof)
    return proof


def update(env, proof):
    content = 'source after Server switch'
    result = env.api(f"/api/ontology/envs/debug/entities/{proof['entity']}/attributes/{proof['source']}/text",
        'PUT',{'format':'md','content':content,'expected_revision':proof['revision']})
    return {**proof,'content':content,'revision':result['revision']}


def verify(env, proof):
    prefix = f"/api/ontology/envs/debug/entities/{proof['entity']}/attributes/{proof['source']}/text"
    current = env.api(prefix+f"/{proof['revision']}")
    assert current['content']==proof['content']
    history = env.api(prefix)['items']
    assert sum(item['is_current'] for item in history)==1
    assert env.api('/api/ontology/envs/debug/graph-aspects')['items'][0]['id']==proof['aspect']
    assert (env.resources.mounts['ontology']/proof['path']).read_text()=='first source'
    assert not (env.resources.mounts['ontology']/'ontology.db').exists()
    status = env.api('/api/ontology/nfs')['status']
    assert status['read_only'] and status['port']==env.resources.ports['ontology']

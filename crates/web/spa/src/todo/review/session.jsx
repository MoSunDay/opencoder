import {Drawer} from 'antd';
import {SessionConversation} from './conversation/session.jsx';

// Secondary entry from the original file/event records.
export function SessionReview({id,sessionId,onClose}) {
  return <Drawer title="会话明细" placement="right" size="85vw" open={!!sessionId} onClose={onClose} destroyOnHidden>
    {sessionId&&<SessionConversation key={`${id}/${sessionId}`} id={id} sessionId={sessionId}/>}
  </Drawer>;
}

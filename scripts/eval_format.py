import hashlib
import json


class SharedDataset:
    def __init__(self):
        self.sessions = {}
        self.histories = {}
        self.questions = []
        self.conflicts = 0

    def add(self, item):
        item = dict(item)
        turns = item.pop('haystack_sessions')
        original_ids = item.pop('haystack_session_ids')
        dates = item.pop('haystack_dates')
        if len(turns) != len(original_ids) or len(turns) != len(dates):
            raise ValueError(f"Unaligned history: {item['question_id']}")
        ids = []
        remap = {}
        for session_id, session in zip(original_ids, turns):
            key = session_id
            if key in self.sessions and self.sessions[key] != session:
                digest = hashlib.sha256(json.dumps(session, sort_keys=True).encode()).hexdigest()
                key = f'{session_id}~{digest}'
                self.conflicts += 1
            if key in self.sessions and self.sessions[key] != session:
                raise ValueError(f'Session id collision: {key}')
            self.sessions.setdefault(key, session)
            ids.append(key)
            remap[session_id] = key
        history = {'session_ids': ids, 'dates': dates}
        history_id = hashlib.sha256(json.dumps(history, sort_keys=True).encode()).hexdigest()
        self.histories.setdefault(history_id, history)
        item['answer_session_ids'] = [remap.get(key, key) for key in item['answer_session_ids']]
        item['history'] = history_id
        self.questions.append(item)

    def write(self, path):
        if len({item['question_id'] for item in self.questions}) != len(self.questions):
            raise ValueError('Duplicate question ids')
        with path.open('w') as output:
            json.dump({'format': 'neomem-eval/2', 'sessions': self.sessions,
                       'histories': self.histories, 'questions': self.questions}, output)
        print(f'{path}: {path.stat().st_size} bytes; {len(self.questions)} questions, '
              f'{len(self.histories)} histories, {len(self.sessions)} sessions; '
              f'{self.conflicts} conflicting session occurrences')

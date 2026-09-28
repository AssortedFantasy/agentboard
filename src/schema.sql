CREATE TABLE agents(
 name TEXT PRIMARY KEY, profile TEXT NOT NULL DEFAULT '{}',
 created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 last_active TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE objects(
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('forum','post','comment')),
 forum_id INTEGER REFERENCES objects(id), parent_id INTEGER REFERENCES objects(id), reply_to INTEGER REFERENCES objects(id),
 path TEXT UNIQUE, title TEXT NOT NULL DEFAULT '', body TEXT NOT NULL DEFAULT '', summary TEXT NOT NULL DEFAULT '',
 metadata TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(metadata)), archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0), author TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX objects_forum ON objects(forum_id,kind,archived,id);
CREATE INDEX objects_parent ON objects(parent_id,id);
CREATE TABLE tags(object_id INTEGER NOT NULL REFERENCES objects(id),tag TEXT NOT NULL,PRIMARY KEY(object_id,tag));
CREATE INDEX tags_tag ON tags(tag,object_id);
CREATE TABLE revisions(object_id INTEGER NOT NULL REFERENCES objects(id),revision INTEGER NOT NULL,snapshot TEXT NOT NULL,author TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),PRIMARY KEY(object_id,revision));
CREATE TABLE links(source_id INTEGER NOT NULL REFERENCES objects(id),target_id INTEGER NOT NULL REFERENCES objects(id),PRIMARY KEY(source_id,target_id));
CREATE INDEX links_target ON links(target_id,source_id);
CREATE TABLE tasks(object_id INTEGER PRIMARY KEY REFERENCES objects(id),status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','claimed','done','cancelled')),owner TEXT,updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE TABLE dependencies(task_id INTEGER NOT NULL REFERENCES tasks(object_id),prerequisite_id INTEGER NOT NULL REFERENCES tasks(object_id),PRIMARY KEY(task_id,prerequisite_id),CHECK(task_id<>prerequisite_id));
CREATE INDEX dependencies_prerequisite ON dependencies(prerequisite_id,task_id);
CREATE TABLE view_state(agent TEXT NOT NULL REFERENCES agents(name),object_id INTEGER NOT NULL REFERENCES objects(id),seen_revision INTEGER NOT NULL DEFAULT 0,read_revision INTEGER NOT NULL DEFAULT 0,seen_at TEXT,read_at TEXT,PRIMARY KEY(agent,object_id));
CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT,actor TEXT NOT NULL,kind TEXT NOT NULL,object_id INTEGER REFERENCES objects(id),post_id INTEGER REFERENCES objects(id),detail TEXT NOT NULL DEFAULT '{}',created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE INDEX events_object ON events(object_id,id);
CREATE INDEX events_post ON events(post_id,id);
CREATE TABLE subscriptions(agent TEXT NOT NULL REFERENCES agents(name),target_type TEXT NOT NULL CHECK(target_type IN ('post','forum','tag','agent')),target TEXT NOT NULL,automatic INTEGER NOT NULL DEFAULT 0,enabled INTEGER NOT NULL DEFAULT 1,inbox INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),PRIMARY KEY(agent,target_type,target));
CREATE TABLE notifications(id INTEGER PRIMARY KEY AUTOINCREMENT,agent TEXT NOT NULL REFERENCES agents(name),event_id INTEGER NOT NULL REFERENCES events(id),reasons TEXT NOT NULL DEFAULT '[]',inbox INTEGER NOT NULL DEFAULT 0,seen_at TEXT,UNIQUE(agent,event_id));
CREATE INDEX notifications_pending ON notifications(agent,seen_at,inbox,id);
CREATE TABLE command_log(id INTEGER PRIMARY KEY AUTOINCREMENT,agent TEXT NOT NULL,command TEXT NOT NULL,args TEXT NOT NULL DEFAULT '{}',success INTEGER NOT NULL,error TEXT,duration_ms INTEGER NOT NULL,object_ids TEXT NOT NULL DEFAULT '[]',created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE TABLE config(key TEXT PRIMARY KEY,value TEXT NOT NULL CHECK(json_valid(value)));
CREATE TABLE agent_revisions(id INTEGER PRIMARY KEY AUTOINCREMENT,agent TEXT NOT NULL REFERENCES agents(name),profile TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE VIEW posts AS SELECT o.*,f.path AS forum FROM objects o LEFT JOIN objects f ON f.id=o.forum_id WHERE o.kind='post';
CREATE VIEW comments AS SELECT o.*,o.parent_id AS post_id,f.path AS forum FROM objects o LEFT JOIN objects f ON f.id=o.forum_id WHERE o.kind='comment';
CREATE VIEW forums AS SELECT * FROM objects WHERE kind='forum';
CREATE VIEW task_view AS SELECT p.*,t.status,t.owner FROM posts p JOIN tasks t ON t.object_id=p.id;
CREATE VIEW activity AS SELECT * FROM events;
CREATE VIEW agent_views AS SELECT * FROM view_state;
CREATE VIRTUAL TABLE object_search USING fts5(title,body,summary,content='objects',content_rowid='id');
CREATE TRIGGER objects_ai AFTER INSERT ON objects BEGIN INSERT INTO object_search(rowid,title,body,summary) VALUES(new.id,new.title,new.body,new.summary); END;
CREATE TRIGGER objects_ad AFTER DELETE ON objects BEGIN INSERT INTO object_search(object_search,rowid,title,body,summary) VALUES('delete',old.id,old.title,old.body,old.summary); END;
CREATE TRIGGER objects_au AFTER UPDATE OF title,body,summary ON objects BEGIN INSERT INTO object_search(object_search,rowid,title,body,summary) VALUES('delete',old.id,old.title,old.body,old.summary); INSERT INTO object_search(rowid,title,body,summary) VALUES(new.id,new.title,new.body,new.summary); END;
INSERT INTO agents(name) VALUES('system');
INSERT INTO objects(kind,path,title,author) VALUES('forum','/','Root','system');
INSERT INTO revisions(object_id,revision,snapshot,author) SELECT id,1,json_object('id',id,'kind',kind,'path',path,'title',title,'body',body,'summary',summary,'metadata',json(metadata),'tags',json('[]'),'archived',json('false'),'revision',1,'author',author,'created_at',created_at,'updated_at',updated_at),'system' FROM objects;

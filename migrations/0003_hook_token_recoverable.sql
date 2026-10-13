-- Hook 地址不再是「只显示一次」的秘密：保存 token 的 AES-256-GCM 密文，
-- 让管理后台可以随时回显、复制完整 Hook 地址；轮换依旧会让旧 token 立刻失效。
--
-- 设计取舍：
--   * 地址（含 token）在 UI 中不再遮蔽 —— 它是运维配置，不是需要藏的密钥；
--   * 但落库仍走主密钥加密（与 lark app_secret 同一套），避免数据库文件被拷走后直接拿到可用地址；
--   * 鉴权/查找继续用 token_hash，密文只服务于展示；
--   * 0003 之前的旧行两列为 NULL，后台会提示「轮换一次即可显示完整地址」。

ALTER TABLE hooks ADD COLUMN token_nonce TEXT;
ALTER TABLE hooks ADD COLUMN token_ciphertext TEXT;

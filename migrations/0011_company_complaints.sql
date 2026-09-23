-- 用人单位被投诉次数：决定其发布职位的展示颜色（投诉越多越暗淡）
ALTER TABLE companies ADD COLUMN complaints_count INTEGER NOT NULL DEFAULT 0;
COMMENT ON COLUMN companies.complaints_count IS '被投诉次数（>=0）；职位列表/详情按此着色（0 最明亮，越多越暗淡）';

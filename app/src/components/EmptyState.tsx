import type { IconName } from "./Icon";
import { Icon } from "./Icon";

interface EmptyStateProps {
  icon: IconName;
  title: string;
  description: string;
}

export function EmptyState({ icon, title, description }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <span className="empty-state__icon"><Icon name={icon} size={23} /></span>
      <strong>{title}</strong>
      <p>{description}</p>
    </div>
  );
}

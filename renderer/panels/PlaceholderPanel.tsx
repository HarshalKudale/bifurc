import React from "react";
import EmptyState from "@/components/ui/EmptyState";

interface Props {
    icon: React.ReactNode;
    title: string;
    description: string;
}

export default function PlaceholderPanel({ icon, title, description }: Props) {
    return (
        <div className="flex-1 flex items-center justify-center">
            <EmptyState
                icon={icon}
                title={title}
                description={description}
                fill
            />
        </div>
    );
}

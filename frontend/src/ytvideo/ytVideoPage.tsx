
import { useState } from 'react';
import {
  Container,
  Title,
  Button,
  Group,
  TextInput,
  Paper,
  Tabs,
  Stack,
  Text,
  Badge,
  ActionIcon,
  Drawer,
} from '@mantine/core';
import { useDisclosure } from '@mantine/hooks';
import { DataTable } from 'mantine-datatable';
import {
  IconSearch,
  IconPlus,
  IconFilter,
  IconEye,
  IconBrandYoutube,
} from '@tabler/icons-react';
import { Visualiser } from './visualiser';

// Mock types matching the visualizer
interface Video {
  id: string;
  title: string;
  channelName: string;
  views: number;
  url: string;
  group: number;
  thumbnail: string;
  status: 'new' | 'processed' | 'archived';
}

const MOCK_VIDEOS: Video[] = Array.from({ length: 50 }).map((_, i) => ({
    id: `v${i}`,
    title: `Video Title ${i} - ${Math.random().toString(36).substring(7)}`,
    channelName: `Channel ${i % 5}`,
    views: Math.floor(Math.random() * 100000),
    url: 'https://youtube.com',
    group: i % 5,
    thumbnail: 'https://via.placeholder.com/150',
    status: ['new', 'processed', 'archived'][i % 3] as any
}));

const MOCK_LINKS = Array.from({ length: 40 }).map((_) => ({
    source: `v${Math.floor(Math.random() * 50)}`,
    target: `v${Math.floor(Math.random() * 50)}`,
    value: 1
}));

export function YTVideoPage() {
  const [activeTab, setActiveTab] = useState<string | null>('list');
  const [videos] = useState<Video[]>(MOCK_VIDEOS);
  const [search, setSearch] = useState('');
  const [selectedVideo, setSelectedVideo] = useState<Video | null>(null);
  const [drawerOpened, { open: openDrawer, close: closeDrawer }] = useDisclosure(false);
  
  // Filter videos
  const filteredVideos = videos.filter((v) =>
    v.title.toLowerCase().includes(search.toLowerCase()) ||
    v.channelName.toLowerCase().includes(search.toLowerCase())
  );

  const handleNodeClick = (node: any) => {
      setSelectedVideo(node as Video);
      openDrawer();
  };

  return (
    <Container size="100%" px={0} h="calc(100vh - 80px)">
       <Stack h="100%" gap={0}>
            <Paper p="md" shadow="xs" style={{ zIndex: 10 }}>
                <Group justify="space-between" mb="sm">
                    <Title order={3}>YouTube Dashboard</Title>
                    <Group>
                        <TextInput 
                            placeholder="Search videos..." 
                            leftSection={<IconSearch size={16} />}
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                        />
                        <Button leftSection={<IconPlus size={16} />}>Add Video</Button>
                    </Group>
                </Group>
                
                <Tabs value={activeTab} onChange={setActiveTab}>
                    <Tabs.List>
                        <Tabs.Tab value="list" leftSection={<IconFilter size={14} />}>List View</Tabs.Tab>
                        <Tabs.Tab value="graph" leftSection={<IconEye size={14} />}>Graph View</Tabs.Tab>
                    </Tabs.List>
                </Tabs>
            </Paper>

            <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
                {activeTab === 'list' && (
                    <DataTable
                        withTableBorder
                        borderRadius="sm"
                        striped
                        highlightOnHover
                        records={filteredVideos}
                        columns={[
                            { accessor: 'title', width: 300 },
                            { accessor: 'channelName', title: 'Channel' },
                            { accessor: 'views', sortable: true },
                            { 
                                accessor: 'status',
                                render: (record) => (
                                    <Badge color={record.status === 'new' ? 'blue' : record.status === 'processed' ? 'green' : 'gray'}>
                                        {record.status}
                                    </Badge>
                                )
                            },
                            {
                                accessor: 'actions',
                                title: '',
                                width: 80,
                                render: () => (
                                    <Group gap={4} wrap="nowrap">
                                        <ActionIcon size="sm" variant="subtle" color="red">
                                            <IconBrandYoutube size={16} />
                                        </ActionIcon>
                                    </Group>
                                )
                            }
                        ]}
                        minHeight={200}
                        height="100%" 
                        // pagination would go here
                    />
                )}

                {activeTab === 'graph' && (
                    <Visualiser 
                        videos={videos} 
                        links={MOCK_LINKS} 
                        onNodeClick={handleNodeClick}
                    />
                )}
            </div>
       </Stack>

       <Drawer 
            opened={drawerOpened} 
            onClose={closeDrawer}
            title={selectedVideo?.title || 'Video Details'}
            position="right"
            size="md"
       >
            {selectedVideo && (
                <Stack>
                    <img src={selectedVideo.thumbnail} alt={selectedVideo.title} style={{ width: '100%', borderRadius: 8 }} />
                    <Title order={4}>{selectedVideo.channelName}</Title>
                    <Text>Views: {selectedVideo.views.toLocaleString()}</Text>
                    <Text>Status: {selectedVideo.status}</Text>
                    <Button component="a" href={selectedVideo.url} target="_blank" leftSection={<IconBrandYoutube size={16}/>}>
                        Watch on YouTube
                    </Button>
                </Stack>
            )}
       </Drawer>
    </Container>
  );
}

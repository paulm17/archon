
import { useState, useEffect, useMemo, useRef } from 'react';
import * as THREE from 'three';
import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { OrbitControls, Billboard, Text } from '@react-three/drei';
import { forceSimulation, forceLink, forceManyBody, forceCenter } from 'd3-force';

interface VideoNode {
  id: string;
  title: string;
  channelName: string;
  views: number;
  group: number;
  url: string;
  thumbnail: string;
  status?: string;
}

interface VideoLink {
  source: string;
  target: string;
  value: number;
}

interface VisualiserProps {
  videos: VideoNode[];
  links: VideoLink[];
  onNodeClick: (node: VideoNode) => void;
}

const CONSTANTS = {
  NODE_REL_SIZE: 4,
  NODE_VAL: 1,
};

function NodeMesh({ node, onNodeClick, isActive }: { node: VideoNode & { x?: number, y?: number, z?: number }, onNodeClick: (node: VideoNode) => void, isActive: boolean }) {
    const meshRef = useRef<THREE.Mesh>(null);
    const [hovered, setHovered] = useState(false);

    useFrame(() => {
        if (meshRef.current && node.x !== undefined && node.y !== undefined) {
             // 2D force layout mapped to 3D plane (z=0) or 3D if simulation allows
             // d3-force is 2D by default unless configured.
             // We'll map x->x, y->y, z->0
             meshRef.current.position.set(node.x, node.y, node.z || 0);
        }
    });

    const color = useMemo(() => {
        const colors = ['#4285F4', '#DB4437', '#F4B400', '#0F9D58', '#AB47BC', '#00ACC1'];
        return colors[node.group % colors.length];
    }, [node.group]);

    return (
        <group>
            <mesh
                ref={meshRef}
                onClick={() => onNodeClick(node)}
                onPointerOver={() => { document.body.style.cursor = 'pointer'; setHovered(true); }}
                onPointerOut={() => { document.body.style.cursor = 'auto'; setHovered(false); }}
            >
                <sphereGeometry args={[CONSTANTS.NODE_VAL * (isActive ? 1.5 : 1) * (hovered ? 1.2 : 1), 32, 32]} />
                <meshStandardMaterial color={color} emissive={color} emissiveIntensity={isActive ? 0.5 : 0.2} />
            </mesh>
            {(hovered || isActive) && meshRef.current && (
                <Billboard position={[meshRef.current.position.x, meshRef.current.position.y + 20, meshRef.current.position.z]}>
                     <Text fontSize={6} color="white" anchorX="center" anchorY="bottom" outlineWidth={0.5} outlineColor="black">
                        {node.title.length > 20 ? node.title.substring(0, 20) + '...' : node.title}
                     </Text>
                </Billboard>
            )}
        </group>
    );
}

function LinkMesh({ link, nodes }: { link: VideoLink & { source: any, target: any }, nodes: any[] }) {
    const lineRef = useRef<THREE.Line>(null);
    
    useFrame(() => {
        if (lineRef.current) {
            const sourceNode = typeof link.source === 'object' ? link.source : nodes.find(n => n.id === link.source);
            const targetNode = typeof link.target === 'object' ? link.target : nodes.find(n => n.id === link.target);

            if (sourceNode && targetNode && sourceNode.x !== undefined && targetNode.x !== undefined) {
                 const points = [
                     new THREE.Vector3(sourceNode.x, sourceNode.y, sourceNode.z || 0),
                     new THREE.Vector3(targetNode.x, targetNode.y, targetNode.z || 0)
                 ];
                 lineRef.current.geometry.setFromPoints(points);
            }
        }
    });

    return (
        <line ref={lineRef as any}>
            <bufferGeometry />
            <lineBasicMaterial color="#999" transparent opacity={0.3} />
        </line>
    );
}

function GraphScene({ videos, links, onNodeClick, activeNodeId }: { videos: VideoNode[], links: VideoLink[], onNodeClick: (n: VideoNode) => void, activeNodeId?: string }) {
    const { camera } = useThree();
    
    // Simulation state
    const simulation = useMemo(() => {
        return forceSimulation()
            .force('link', forceLink().id((d: any) => d.id).distance(50))
            .force('charge', forceManyBody().strength(-200))
            .force('center', forceCenter(0, 0))
            .stop();
    }, []);

    useEffect(() => {
        // Initialize simulation
        simulation.nodes(videos as any);
        simulation.force('link', forceLink(links).id((d: any) => d.id).distance(50));
        simulation.restart();
        
        // Warm up
        for (let i = 0; i < 100; ++i) simulation.tick();
        
    }, [videos, links, simulation]);

    useFrame(() => {
        simulation.tick();
    });
    
    // Initial camera position
    useEffect(() => {
        camera.position.z = 800;
    }, [camera]);

    return (
        <>
            <ambientLight intensity={0.5} />
            <pointLight position={[10, 10, 10]} />
            <OrbitControls />
            
            {links.map((link, i) => (
                 <LinkMesh key={i} link={link as any} nodes={videos} />
            ))}

            {videos.map((node: any) => (
                <NodeMesh 
                    key={node.id} 
                    node={node} 
                    onNodeClick={onNodeClick} 
                    isActive={node.id === activeNodeId}
                />
            ))}
        </>
    );
}

export function Visualiser({ videos, links, onNodeClick }: VisualiserProps) {
    const [activeNode, setActiveNode] = useState<VideoNode | null>(null);

    const handleNodeClick = (node: VideoNode) => {
        setActiveNode(node);
        onNodeClick(node);
    };

    return (
        <div style={{ width: '100%', height: '100%', background: '#000000' }}>
            <Canvas>
                <GraphScene 
                    videos={videos} 
                    links={links} 
                    onNodeClick={handleNodeClick}
                    activeNodeId={activeNode?.id}
                />
            </Canvas>
        </div>
    );
}
